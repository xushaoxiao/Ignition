package attribution

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/shaoxiaoxu/linksprout/internal/dao"
	"github.com/shaoxiaoxu/linksprout/internal/models"
	"github.com/shaoxiaoxu/linksprout/internal/service/risk"
)

var (
	ErrCodeNotFound  = errors.New("attribution: 领奖码不存在")
	ErrCodeUsed      = errors.New("attribution: 领奖码已被核销")
	ErrCodeExpired   = errors.New("attribution: 领奖码已过期")
	ErrCodeMalformed = errors.New("attribution: 领奖码格式非法")
	ErrRiskDenied    = errors.New("attribution: 风控拒绝")
	ErrAlreadyBound  = errors.New("attribution: 该 App 用户已归属其他渠道")
)

// RedeemRequest 核销请求。
type RedeemRequest struct {
	TenantID  int64
	Code      string
	AppUserID string
	DeviceID  string
	IP        string
	Now       time.Time
}

// RedeemResult 核销结果，回给主 App。
type RedeemResult struct {
	Attributed   bool                     `json:"attributed"`
	KOLID        int64                    `json:"kol_id,omitempty"`
	CampaignID   int64                    `json:"campaign_id,omitempty"`
	Method       models.AttributionMethod `json:"method,omitempty"`
	PolicyVersion string                  `json:"policy_version,omitempty"`
	// Held 为 true 表示归因成立、奖励可发，但该笔转化被风控暂缓计费。
	// 主 App 不需要区别对待，这个字段只是为了让客户侧日志能对上账。
	Held bool `json:"held"`
}

// Service 归因服务。
type Service struct {
	db     *dao.DB
	policy Policy
}

func NewService(db *dao.DB, p Policy) *Service { return &Service{db: db, policy: p} }

// Redeem 核销领奖码，是整条链路唯一的"缝合点"：TG 侧身份（tg_user_id）
// 与 App 侧身份（app_user_id）在这一刻绑定，归因与可计费事件同时产生。
//
// 全过程必须在单个事务内完成。如果绑定成功但归因写入失败，这个用户就永远
// 无法被正确归因了 —— 领奖码已作废，没有第二次机会。
func (s *Service) Redeem(ctx context.Context, req RedeemRequest) (*RedeemResult, error) {
	code := NormalizeClaimCode(req.Code)
	if !ValidClaimCodeFormat(code) {
		return nil, ErrCodeMalformed
	}
	if req.Now.IsZero() {
		req.Now = time.Now()
	}

	var out RedeemResult
	err := s.db.InTenantTx(ctx, req.TenantID, func(tx pgx.Tx) error {
		var (
			claimID    int64
			playerID   int64
			campaignID int64
			linkID     int64
			kolID      int64
			status     models.ClaimStatus
			issuedAt   time.Time
			expiresAt  time.Time
			tgUserID   int64
		)
		// FOR UPDATE 锁住这一行：同一个码被并发提交两次时（用户狂点、客户端
		// 重试），第二个事务会阻塞到第一个提交后再读，从而看到 redeemed 状态。
		// 没有这把锁，两个事务都会读到 issued 并各写一条归因和一笔计费。
		err := tx.QueryRow(ctx, `
			SELECT c.id, c.player_id, c.campaign_id, c.link_id, c.kol_id,
			       c.status, c.issued_at, c.expires_at, p.tg_user_id
			  FROM claim_code c
			  JOIN player p ON p.id = c.player_id
			 WHERE c.code = $1
			 FOR UPDATE OF c`, code).
			Scan(&claimID, &playerID, &campaignID, &linkID, &kolID,
				&status, &issuedAt, &expiresAt, &tgUserID)
		if errors.Is(err, pgx.ErrNoRows) {
			return ErrCodeNotFound
		}
		if err != nil {
			return fmt.Errorf("查询领奖码: %w", err)
		}

		switch {
		case status == models.ClaimRedeemed:
			return ErrCodeUsed
		case status != models.ClaimIssued:
			return ErrCodeNotFound
		case req.Now.After(expiresAt):
			return ErrCodeExpired
		}

		// ---- 风控 L1 ----
		var devicePlayerCount, ipRedeemToday int
		if req.DeviceID != "" {
			if err := tx.QueryRow(ctx,
				`SELECT count(*) FROM player WHERE $1 = ANY(device_ids)`,
				req.DeviceID).Scan(&devicePlayerCount); err != nil {
				return fmt.Errorf("统计设备绑定数: %w", err)
			}
		}
		if req.IP != "" {
			if err := tx.QueryRow(ctx, `
				SELECT count(*) FROM risk_signal
				 WHERE stage = 'redeem' AND ip = $1::inet
				   AND created_at >= date_trunc('day', $2::timestamptz)`,
				req.IP, req.Now).Scan(&ipRedeemToday); err != nil {
				return fmt.Errorf("统计 IP 核销数: %w", err)
			}
		}
		verdict := risk.CheckRedeem(risk.RedeemInput{
			DevicePlayerCount: devicePlayerCount,
			IPRedeemToday:     ipRedeemToday,
			TGUserID:          tgUserID,
			ClickToRedeem:     req.Now.Sub(issuedAt),
		})
		if verdict.Action == risk.ActionDeny {
			return fmt.Errorf("%w: %s", ErrRiskDenied, verdict.Rule)
		}

		// ---- 状态推进与身份绑定 ----
		if _, err := tx.Exec(ctx,
			`UPDATE claim_code SET status = 'redeemed', redeemed_at = $2 WHERE id = $1`,
			claimID, req.Now); err != nil {
			return fmt.Errorf("核销领奖码: %w", err)
		}

		// app_user_id 上有唯一索引。若该 App 用户已被别的 Player 绑定，
		// 说明同一个 App 账号在尝试领取第二份归因 —— 冲突即拒绝。
		ct, err := tx.Exec(ctx, `
			UPDATE player
			   SET app_user_id = $2,
			       device_ids  = CASE WHEN $3 = '' OR $3 = ANY(device_ids)
			                          THEN device_ids ELSE array_append(device_ids, $3) END
			 WHERE id = $1
			   AND (app_user_id IS NULL OR app_user_id = $2)`,
			playerID, req.AppUserID, req.DeviceID)
		if err != nil {
			return fmt.Errorf("绑定 App 用户: %w", err)
		}
		if ct.RowsAffected() == 0 {
			return ErrAlreadyBound
		}

		// ---- 归因 ----
		method := models.MethodDeterministicCode
		evidence, _ := json.Marshal(map[string]any{
			"claim_code":  code,
			"claim_id":    claimID,
			"app_user_id": req.AppUserID,
			"device_id":   req.DeviceID,
			"ip":          req.IP,
			"issued_at":   issuedAt,
			"redeemed_at": req.Now,
			"risk":        verdict,
		})

		var attributionID int64
		err = tx.QueryRow(ctx, `
			INSERT INTO attribution
			  (tenant_id, player_id, kol_id, campaign_id, link_id, method, confidence,
			   is_billable, policy_version, touch_at, attributed_at, locked_until, evidence)
			VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
			ON CONFLICT (tenant_id, player_id) DO NOTHING
			RETURNING id`,
			req.TenantID, playerID, kolID, campaignID, linkID,
			method, method.Confidence(), method.IsBillable(), s.policy.Version,
			issuedAt, req.Now, req.Now.Add(s.policy.LockPeriod), evidence).
			Scan(&attributionID)
		if errors.Is(err, pgx.ErrNoRows) {
			// 单归因约束：该 Player 已归属某个 KOL。归因锁定期内不改判，
			// 沿用既有归因，但不再产生新的可计费事件。
			if err := tx.QueryRow(ctx,
				`SELECT id, kol_id, campaign_id FROM attribution WHERE player_id = $1`,
				playerID).Scan(&attributionID, &out.KOLID, &out.CampaignID); err != nil {
				return fmt.Errorf("读取既有归因: %w", err)
			}
			out.Attributed = true
			out.Method = method
			out.PolicyVersion = s.policy.Version
			return nil
		}
		if err != nil {
			return fmt.Errorf("写入归因: %w", err)
		}

		// ---- 可计费事件 ----
		// 只有 is_billable 的归因才产生计费事件（约束 C1）。
		if method.IsBillable() {
			status := models.StatusPending
			reason := ""
			if verdict.Action == risk.ActionHold {
				status = models.StatusHeld
				reason = verdict.Rule
			}
			// external_id 用 claim_id：核销是我方可确认的事实，不依赖客户回传，
			// 因此 MVP 阶段的 CPA 计费不存在"客户少报"的漏损。
			if _, err := tx.Exec(ctx, `
				INSERT INTO billable_event
				  (tenant_id, attribution_id, event_type, external_id, status,
				   amount_cents, currency, occurred_at, received_at, hold_until, status_reason)
				VALUES ($1,$2,$3,$4,$5,
				        COALESCE((SELECT (cpa_rates->>$3)::bigint FROM pricing_config
				                   WHERE (tenant_id = $1 OR tenant_id IS NULL)
				                     AND effective_from <= $6
				                     AND (effective_to IS NULL OR effective_to > $6)
				                   -- 租户专属定价优先于全局默认
				                   ORDER BY tenant_id NULLS LAST, effective_from DESC
				                   LIMIT 1), 0),
				        'USD', $6, $6, $7, $8)
				ON CONFLICT (tenant_id, event_type, external_id) DO NOTHING`,
				req.TenantID, attributionID, models.EventActivation,
				fmt.Sprintf("claim:%d", claimID), status,
				req.Now, req.Now.Add(s.policy.HoldPeriod(models.EventActivation)), reason,
			); err != nil {
				return fmt.Errorf("写入可计费事件: %w", err)
			}
			out.Held = status == models.StatusHeld
		}

		// ---- L2 信号采集：只存不判，这些数据事后无法补采 ----
		if _, err := tx.Exec(ctx, `
			INSERT INTO risk_signal (tenant_id, player_id, stage, ip, device_id, latency_ms, attrs)
			VALUES ($1,$2,'redeem', NULLIF($3,'')::inet, $4, $5, $6)`,
			req.TenantID, playerID, req.IP, req.DeviceID,
			req.Now.Sub(issuedAt).Milliseconds(),
			mustJSON(map[string]any{"rule": verdict.Rule, "action": verdict.Action}),
		); err != nil {
			return fmt.Errorf("写入风控信号: %w", err)
		}

		out.Attributed = true
		out.KOLID = kolID
		out.CampaignID = campaignID
		out.Method = method
		out.PolicyVersion = s.policy.Version
		return nil
	})
	if err != nil {
		return nil, err
	}
	return &out, nil
}

func mustJSON(v any) []byte {
	b, err := json.Marshal(v)
	if err != nil {
		return []byte(`{}`)
	}
	return b
}
