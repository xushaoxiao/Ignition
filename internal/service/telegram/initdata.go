// Package telegram 实现 Telegram Mini App 的 initData 校验。
//
// 这是系统的第一道信任边界：initData 决定了"这个请求来自哪个 TG 用户"，
// 而该身份最终会通过领奖码核销绑定到可计费的归因上。校验一旦被绕过，
// 整条收入链路都不可信。
package telegram

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"sort"
	"strconv"
	"strings"
	"time"
)

var (
	ErrMissingHash  = errors.New("telegram: initData 缺少 hash")
	ErrBadSignature = errors.New("telegram: initData 签名不匹配")
	ErrExpired      = errors.New("telegram: initData 已过期")
	ErrMissingUser  = errors.New("telegram: initData 缺少 user")
)

// DefaultMaxAge initData 的最大接受时效。
//
// Telegram 不会主动使 initData 失效，所以时效完全由我们把关。取 5 分钟：
// 足够覆盖正常的网络与用户操作延迟，又能把被截获的 initData 的可重放窗口
// 压到很小。校验通过后应立即换发自己的短期 JWT，后续请求不再重放 initData。
const DefaultMaxAge = 5 * time.Minute

// User 是 initData 里的用户信息。字段名对应 Telegram 的 JSON。
type User struct {
	ID           int64  `json:"id"`
	FirstName    string `json:"first_name"`
	LastName     string `json:"last_name"`
	Username     string `json:"username"`
	LanguageCode string `json:"language_code"`
	IsPremium    bool   `json:"is_premium"`
	PhotoURL     string `json:"photo_url"`
}

// InitData 是校验通过后的 initData 内容。
type InitData struct {
	User      User
	AuthDate  time.Time
	StartParam string // ?startapp= 的值，即 tracking_id
	QueryID   string
	Raw       url.Values
}

// Verify 校验 initData 的签名与时效，返回其内容。
//
// 算法（Telegram 官方定义）：
//  1. 取出并移除 hash 字段
//  2. 其余字段按 key 升序排成 "k=v" 并用 \n 连接，得到 data_check_string
//  3. secret = HMAC-SHA256(key="WebAppData", data=bot_token)
//  4. 期望 hash = HMAC-SHA256(key=secret, data=data_check_string)
//
// 多租户注意：每个租户用自己的 Bot，调用方必须先定位租户、取对应 token，
// 不能用平台级的单一 token 校验。
func Verify(rawInitData, botToken string, maxAge time.Duration) (*InitData, error) {
	values, err := url.ParseQuery(rawInitData)
	if err != nil {
		return nil, fmt.Errorf("telegram: initData 解析失败: %w", err)
	}

	givenHash := values.Get("hash")
	if givenHash == "" {
		return nil, ErrMissingHash
	}
	values.Del("hash")
	// signature 是 Telegram 的 Ed25519 第三方校验字段，不参与 HMAC 计算。
	values.Del("signature")

	keys := make([]string, 0, len(values))
	for k := range values {
		keys = append(keys, k)
	}
	sort.Strings(keys)

	var sb strings.Builder
	for i, k := range keys {
		if i > 0 {
			sb.WriteByte('\n')
		}
		sb.WriteString(k)
		sb.WriteByte('=')
		sb.WriteString(values.Get(k))
	}

	secret := hmacSHA256([]byte("WebAppData"), []byte(botToken))
	expected := hmacSHA256(secret, []byte(sb.String()))

	// 定长比较，避免通过响应时间侧信道逐字节爆破 hash。
	if !hmac.Equal([]byte(hex.EncodeToString(expected)), []byte(givenHash)) {
		return nil, ErrBadSignature
	}

	authDateUnix, err := strconv.ParseInt(values.Get("auth_date"), 10, 64)
	if err != nil {
		return nil, fmt.Errorf("telegram: auth_date 非法: %w", err)
	}
	authDate := time.Unix(authDateUnix, 0)
	if maxAge > 0 && time.Since(authDate) > maxAge {
		return nil, ErrExpired
	}

	userJSON := values.Get("user")
	if userJSON == "" {
		return nil, ErrMissingUser
	}
	var u User
	if err := json.Unmarshal([]byte(userJSON), &u); err != nil {
		return nil, fmt.Errorf("telegram: user 解析失败: %w", err)
	}
	if u.ID == 0 {
		return nil, ErrMissingUser
	}

	return &InitData{
		User:       u,
		AuthDate:   authDate,
		StartParam: values.Get("start_param"),
		QueryID:    values.Get("query_id"),
		Raw:        values,
	}, nil
}

func hmacSHA256(key, data []byte) []byte {
	m := hmac.New(sha256.New, key)
	m.Write(data)
	return m.Sum(nil)
}
