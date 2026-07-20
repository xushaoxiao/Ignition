// Package hmacsig 实现主 App 回传（S2S postback）的签名与校验。
//
// 回传接口是唯一由客户主动调用、且直接产生可计费事件的入口，必须同时防伪造
// 和防重放。
package hmacsig

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"strconv"
	"time"
)

var (
	ErrBadSignature = errors.New("hmacsig: 签名不匹配")
	ErrStaleRequest = errors.New("hmacsig: 时间戳超出允许窗口")
	ErrBadTimestamp = errors.New("hmacsig: 时间戳格式非法")
)

// DefaultSkew 允许的时钟偏移窗口。
//
// 双向 5 分钟：客户服务器的时钟未必与我们同步，窗口太窄会造成大量误拒
// （表现为客户"回传丢失"，直接影响收入统计）；太宽则放大重放窗口。
const DefaultSkew = 5 * time.Minute

// Sign 计算签名：HMAC-SHA256(secret, timestamp + "." + body)。
//
// 时间戳纳入签名范围，使其无法被篡改 —— 否则攻击者可以拿一个旧请求改时间戳
// 后无限重放。
func Sign(secret []byte, timestamp int64, body []byte) string {
	m := hmac.New(sha256.New, secret)
	m.Write([]byte(strconv.FormatInt(timestamp, 10)))
	m.Write([]byte("."))
	m.Write(body)
	return hex.EncodeToString(m.Sum(nil))
}

// Verify 校验签名与时间戳窗口。
//
// 注意：这里只挡住重放的"时间窗"这一半，另一半靠 billable_event 上
// (tenant_id, event_type, external_id) 的唯一约束 —— 窗口内的重放会被幂等
// 吃掉，不会产生第二笔计费。两者缺一不可。
func Verify(secret []byte, timestampHeader, signature string, body []byte, skew time.Duration) error {
	ts, err := strconv.ParseInt(timestampHeader, 10, 64)
	if err != nil {
		return fmt.Errorf("%w: %q", ErrBadTimestamp, timestampHeader)
	}
	if d := time.Since(time.Unix(ts, 0)); d > skew || d < -skew {
		return ErrStaleRequest
	}
	expected := Sign(secret, ts, body)
	if !hmac.Equal([]byte(expected), []byte(signature)) {
		return ErrBadSignature
	}
	return nil
}
