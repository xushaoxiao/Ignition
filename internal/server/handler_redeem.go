package server

import (
	"encoding/json"
	"errors"
	"net/http"
	"strconv"
	"time"

	"github.com/shaoxiaoxu/linksprout/internal/service/attribution"
)

func (s *Server) handleHealth(w http.ResponseWriter, r *http.Request) {
	if err := s.db.Pool.Ping(r.Context()); err != nil {
		writeErr(w, http.StatusServiceUnavailable, "db_unavailable", "数据库不可用", true)
		return
	}
	writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
}

type redeemRequestBody struct {
	ClaimCode string `json:"claim_code"`
	AppUserID string `json:"app_user_id"`
	DeviceID  string `json:"device_id"`
}

// handleRedeem 处理领奖码核销。
//
// 这是计费链路的关键路径，SLO 高于游戏链路：它挂了等于客户的用户领不到奖，
// 损害的是客户的客户。
func (s *Server) handleRedeem(w http.ResponseWriter, r *http.Request) {
	tenantID, ok := s.authTenant(w, r)
	if !ok {
		return
	}

	var body redeemRequestBody
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, 8<<10)).Decode(&body); err != nil {
		writeErr(w, http.StatusBadRequest, "bad_request", "请求体解析失败", false)
		return
	}
	if body.ClaimCode == "" || body.AppUserID == "" {
		writeErr(w, http.StatusBadRequest, "bad_request",
			"claim_code 与 app_user_id 必填", false)
		return
	}

	res, err := s.attr.Redeem(r.Context(), attribution.RedeemRequest{
		TenantID:  tenantID,
		Code:      body.ClaimCode,
		AppUserID: body.AppUserID,
		DeviceID:  body.DeviceID,
		IP:        clientIP(r),
		Now:       time.Now(),
	})
	if err != nil {
		s.writeRedeemError(w, err)
		return
	}
	writeJSON(w, http.StatusOK, res)
}

// writeRedeemError 把领域错误映射为对客户有用的 HTTP 响应。
//
// 映射原则：只有真正瞬时的故障才标记 retryable。领奖码不存在、已用、过期
// 都是终态，客户重试只会放大无效流量。
func (s *Server) writeRedeemError(w http.ResponseWriter, err error) {
	switch {
	case errors.Is(err, attribution.ErrCodeMalformed):
		writeErr(w, http.StatusBadRequest, "code_malformed", "领奖码格式非法", false)
	case errors.Is(err, attribution.ErrCodeNotFound):
		writeErr(w, http.StatusNotFound, "code_not_found", "领奖码不存在", false)
	case errors.Is(err, attribution.ErrCodeUsed):
		writeErr(w, http.StatusConflict, "code_used", "领奖码已被核销", false)
	case errors.Is(err, attribution.ErrCodeExpired):
		writeErr(w, http.StatusGone, "code_expired", "领奖码已过期", false)
	case errors.Is(err, attribution.ErrAlreadyBound):
		writeErr(w, http.StatusConflict, "already_bound", "该 App 用户已归属其他渠道", false)
	case errors.Is(err, attribution.ErrRiskDenied):
		writeErr(w, http.StatusForbidden, "risk_denied", "风控拒绝", false)
	default:
		s.log.Error("redeem failed", "err", err)
		writeErr(w, http.StatusInternalServerError, "internal_error", "内部错误", true)
	}
}

// authTenant 解析并校验调用方身份。
//
// MVP 占位实现：从 X-Tenant-ID 读取。上线前必须换成 API Key + HMAC 签名，
// 否则任何人都能伪造租户身份核销任意领奖码。
//
// TODO(auth): 接入 API Key 校验后移除对 X-Tenant-ID 的信任。
func (s *Server) authTenant(w http.ResponseWriter, r *http.Request) (int64, bool) {
	raw := r.Header.Get("X-Tenant-ID")
	if raw == "" {
		writeErr(w, http.StatusUnauthorized, "unauthorized", "缺少租户标识", false)
		return 0, false
	}
	id, err := strconv.ParseInt(raw, 10, 64)
	if err != nil || id <= 0 {
		writeErr(w, http.StatusUnauthorized, "unauthorized", "租户标识非法", false)
		return 0, false
	}
	return id, true
}
