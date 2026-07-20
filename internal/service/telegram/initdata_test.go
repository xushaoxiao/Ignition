package telegram_test

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"net/url"
	"sort"
	"strings"
	"testing"
	"time"

	"github.com/shaoxiaoxu/linksprout/internal/service/telegram"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

const testBotToken = "123456:AAH-test-bot-token"

// signInitData 按 Telegram 官方算法构造一份合法的 initData，用于测试。
func signInitData(t *testing.T, token string, fields map[string]string) string {
	t.Helper()
	keys := make([]string, 0, len(fields))
	for k := range fields {
		keys = append(keys, k)
	}
	sort.Strings(keys)

	parts := make([]string, 0, len(keys))
	for _, k := range keys {
		parts = append(parts, k+"="+fields[k])
	}
	secret := hmacSum([]byte("WebAppData"), []byte(token))
	h := hex.EncodeToString(hmacSum(secret, []byte(strings.Join(parts, "\n"))))

	v := url.Values{}
	for k, val := range fields {
		v.Set(k, val)
	}
	v.Set("hash", h)
	return v.Encode()
}

func hmacSum(key, data []byte) []byte {
	m := hmac.New(sha256.New, key)
	m.Write(data)
	return m.Sum(nil)
}

func validFields(authDate time.Time) map[string]string {
	return map[string]string{
		"auth_date":   fmt.Sprint(authDate.Unix()),
		"query_id":    "AAH123",
		"start_param": "aB3xY9zK1m",
		"user":        `{"id":777001,"first_name":"Dave","username":"dave","is_premium":true}`,
	}
}

func TestVerifyAcceptsValidInitData(t *testing.T) {
	raw := signInitData(t, testBotToken, validFields(time.Now()))

	got, err := telegram.Verify(raw, testBotToken, telegram.DefaultMaxAge)

	require.NoError(t, err)
	assert.EqualValues(t, 777001, got.User.ID)
	assert.Equal(t, "dave", got.User.Username)
	assert.True(t, got.User.IsPremium)
	assert.Equal(t, "aB3xY9zK1m", got.StartParam, "start_param 即 tracking_id")
}

func TestVerifyRejectsWrongToken(t *testing.T) {
	raw := signInitData(t, testBotToken, validFields(time.Now()))

	_, err := telegram.Verify(raw, "999999:another-tenant-token", telegram.DefaultMaxAge)

	assert.ErrorIs(t, err, telegram.ErrBadSignature)
}

// 篡改任意字段都必须导致签名失败 —— 否则攻击者可以改 start_param
// 把归因转给别的 KOL。
func TestVerifyRejectsTamperedField(t *testing.T) {
	raw := signInitData(t, testBotToken, validFields(time.Now()))
	tampered := strings.Replace(raw, "aB3xY9zK1m", "ATTACKER01", 1)
	require.NotEqual(t, raw, tampered)

	_, err := telegram.Verify(tampered, testBotToken, telegram.DefaultMaxAge)

	assert.ErrorIs(t, err, telegram.ErrBadSignature)
}

// Telegram 不会主动使 initData 失效，时效完全由我们把关。
func TestVerifyRejectsExpired(t *testing.T) {
	raw := signInitData(t, testBotToken, validFields(time.Now().Add(-30*time.Minute)))

	_, err := telegram.Verify(raw, testBotToken, telegram.DefaultMaxAge)

	assert.ErrorIs(t, err, telegram.ErrExpired)
}

func TestVerifyRejectsMissingHash(t *testing.T) {
	_, err := telegram.Verify("auth_date=1&user=%7B%7D", testBotToken, telegram.DefaultMaxAge)

	assert.ErrorIs(t, err, telegram.ErrMissingHash)
}

func TestVerifyRejectsMissingUser(t *testing.T) {
	fields := map[string]string{"auth_date": fmt.Sprint(time.Now().Unix())}
	raw := signInitData(t, testBotToken, fields)

	_, err := telegram.Verify(raw, testBotToken, telegram.DefaultMaxAge)

	assert.ErrorIs(t, err, telegram.ErrMissingUser)
}

// signature 是 Telegram 的 Ed25519 第三方校验字段，不参与 HMAC 计算。
// 若未从 data_check_string 中剔除，所有带该字段的真实请求都会被误拒。
func TestVerifyIgnoresSignatureField(t *testing.T) {
	raw := signInitData(t, testBotToken, validFields(time.Now()))
	raw += "&signature=" + url.QueryEscape("ed25519-third-party-sig")

	_, err := telegram.Verify(raw, testBotToken, telegram.DefaultMaxAge)

	assert.NoError(t, err)
}
