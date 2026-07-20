package hmacsig_test

import (
	"fmt"
	"testing"
	"time"

	"github.com/shaoxiaoxu/linksprout/pkg/hmacsig"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

var secret = []byte("tenant-postback-secret")

func TestVerifyAcceptsValid(t *testing.T) {
	body := []byte(`{"app_user_id":"u1","transaction_id":"t1","amount":999}`)
	ts := time.Now().Unix()
	sig := hmacsig.Sign(secret, ts, body)

	err := hmacsig.Verify(secret, fmt.Sprint(ts), sig, body, hmacsig.DefaultSkew)

	require.NoError(t, err)
}

func TestVerifyRejectsTamperedBody(t *testing.T) {
	body := []byte(`{"amount":999}`)
	ts := time.Now().Unix()
	sig := hmacsig.Sign(secret, ts, body)

	err := hmacsig.Verify(secret, fmt.Sprint(ts), sig, []byte(`{"amount":99900}`), hmacsig.DefaultSkew)

	assert.ErrorIs(t, err, hmacsig.ErrBadSignature)
}

func TestVerifyRejectsWrongSecret(t *testing.T) {
	body := []byte(`{"amount":999}`)
	ts := time.Now().Unix()
	sig := hmacsig.Sign(secret, ts, body)

	err := hmacsig.Verify([]byte("other-tenant-secret"), fmt.Sprint(ts), sig, body, hmacsig.DefaultSkew)

	assert.ErrorIs(t, err, hmacsig.ErrBadSignature)
}

// 时间戳纳入签名范围，改了时间戳签名就对不上 —— 攻击者无法拿旧请求
// 改时间戳后无限重放。
func TestVerifyRejectsTamperedTimestamp(t *testing.T) {
	body := []byte(`{"amount":999}`)
	ts := time.Now().Unix()
	sig := hmacsig.Sign(secret, ts, body)

	err := hmacsig.Verify(secret, fmt.Sprint(ts+1), sig, body, hmacsig.DefaultSkew)

	assert.ErrorIs(t, err, hmacsig.ErrBadSignature)
}

func TestVerifyRejectsStale(t *testing.T) {
	body := []byte(`{"amount":999}`)
	ts := time.Now().Add(-30 * time.Minute).Unix()
	sig := hmacsig.Sign(secret, ts, body)

	err := hmacsig.Verify(secret, fmt.Sprint(ts), sig, body, hmacsig.DefaultSkew)

	assert.ErrorIs(t, err, hmacsig.ErrStaleRequest)
}

// 窗口是双向的：客户服务器时钟快于我们时同样要拒绝，否则攻击者可以
// 签一个未来的时间戳换取一个超长的有效期。
func TestVerifyRejectsFutureBeyondSkew(t *testing.T) {
	body := []byte(`{"amount":999}`)
	ts := time.Now().Add(30 * time.Minute).Unix()
	sig := hmacsig.Sign(secret, ts, body)

	err := hmacsig.Verify(secret, fmt.Sprint(ts), sig, body, hmacsig.DefaultSkew)

	assert.ErrorIs(t, err, hmacsig.ErrStaleRequest)
}

func TestVerifyRejectsBadTimestampFormat(t *testing.T) {
	err := hmacsig.Verify(secret, "not-a-number", "deadbeef", []byte(`{}`), hmacsig.DefaultSkew)

	assert.ErrorIs(t, err, hmacsig.ErrBadTimestamp)
}
