package attribution_test

import (
	"strings"
	"testing"

	"github.com/shaoxiaoxu/linksprout/internal/service/attribution"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

// 领奖码字符集必须排除易混字符：手动输入是 iOS 侧唯一可计费的归因路径，
// 一个字符的误读就是一次收入损失。
func TestNewClaimCodeExcludesConfusables(t *testing.T) {
	for i := 0; i < 500; i++ {
		code, err := attribution.NewClaimCode()
		require.NoError(t, err)
		require.Len(t, code, 8)
		for _, bad := range []string{"0", "O", "1", "I", "L"} {
			assert.NotContains(t, code, bad, "码 %q 含易混字符 %s", code, bad)
		}
	}
}

func TestNewClaimCodeIsRandom(t *testing.T) {
	seen := make(map[string]bool, 200)
	for i := 0; i < 200; i++ {
		code, err := attribution.NewClaimCode()
		require.NoError(t, err)
		assert.False(t, seen[code], "重复的领奖码 %q", code)
		seen[code] = true
	}
}

// tracking_id 必须不可枚举：能猜出别人的链接就能互相刷量或窃取归因。
func TestNewTrackingIDIsUnpredictable(t *testing.T) {
	seen := make(map[string]bool, 300)
	for i := 0; i < 300; i++ {
		id, err := attribution.NewTrackingID()
		require.NoError(t, err)
		require.Len(t, id, 10)
		assert.False(t, seen[id], "重复的 tracking_id %q", id)
		seen[id] = true
	}
}

func TestNormalizeClaimCode(t *testing.T) {
	cases := map[string]string{
		"ab3xy9zk":   "AB3XY9ZK",
		"AB3X-Y9ZK":  "AB3XY9ZK",
		"ab3x y9zk":  "AB3XY9ZK",
		" AB3XY9ZK ": "AB3XY9ZK",
		"ab3x_y9zk":  "AB3XY9ZK",
	}
	for in, want := range cases {
		assert.Equal(t, want, attribution.NormalizeClaimCode(in), "输入 %q", in)
	}
}

// 刻意不做混淆字符映射：把 "0" 猜成 "O" 或 "D" 都可能把一个有效码变成
// 另一个有效码，从而归到错误的 KOL 名下。宁可报格式错让用户重输。
func TestNormalizeDoesNotGuessConfusables(t *testing.T) {
	got := attribution.NormalizeClaimCode("AB0XY1ZK")

	assert.Equal(t, "AB0XY1ZK", got, "不应擅自替换 0 和 1")
	assert.False(t, attribution.ValidClaimCodeFormat(got), "应被格式校验拒绝")
}

func TestValidClaimCodeFormat(t *testing.T) {
	code, err := attribution.NewClaimCode()
	require.NoError(t, err)
	assert.True(t, attribution.ValidClaimCodeFormat(code))

	assert.False(t, attribution.ValidClaimCodeFormat(""), "空码")
	assert.False(t, attribution.ValidClaimCodeFormat("AB3XY9Z"), "长度不足")
	assert.False(t, attribution.ValidClaimCodeFormat("AB3XY9ZKQ"), "长度超出")
	assert.False(t, attribution.ValidClaimCodeFormat("ab3xy9zk"), "小写未规整")
	assert.False(t, attribution.ValidClaimCodeFormat(strings.Repeat("0", 8)), "含集外字符")
}
