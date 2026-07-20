package risk_test

import (
	"testing"
	"time"

	"github.com/shaoxiaoxu/linksprout/internal/service/risk"
	"github.com/stretchr/testify/assert"
)

func TestCheckPlayEnforcesDailyLimit(t *testing.T) {
	v := risk.CheckPlay(risk.PlayInput{TodayPlayCount: 3, DailyPlayLimit: 3})
	assert.Equal(t, risk.ActionDeny, v.Action)
	assert.Equal(t, "daily_play_limit", v.Rule)

	v = risk.CheckPlay(risk.PlayInput{TodayPlayCount: 2, DailyPlayLimit: 3})
	assert.Equal(t, risk.ActionPass, v.Action)
}

// 核销环节的核心取舍：尽量只 hold 不 deny。
// 误杀一个真实用户不可挽回；放过一个刷子只是暂时多算一笔，冷静期内可冲正。
func TestCheckRedeemPrefersHoldOverDeny(t *testing.T) {
	holdCases := []struct {
		name string
		in   risk.RedeemInput
		rule string
	}{
		{"IP 核销过频", risk.RedeemInput{IPRedeemToday: 10}, "ip_redeem_rate"},
		{"耗时过短", risk.RedeemInput{ClickToRedeem: 500 * time.Millisecond}, "too_fast"},
		{"新注册账号", risk.RedeemInput{TGUserID: risk.NewAccountThreshold + 1}, "new_tg_account"},
	}
	for _, c := range holdCases {
		t.Run(c.name, func(t *testing.T) {
			v := risk.CheckRedeem(c.in)
			assert.Equal(t, risk.ActionHold, v.Action, "应暂缓而非拒绝")
			assert.Equal(t, c.rule, v.Rule)
		})
	}
}

// 唯一直接拒绝的维度：一台设备绑定过多账号是明确的养号特征，
// 真实用户几乎不可能触发。
func TestCheckRedeemDeniesDeviceFarming(t *testing.T) {
	v := risk.CheckRedeem(risk.RedeemInput{DevicePlayerCount: risk.MaxPlayersPerDevice})

	assert.Equal(t, risk.ActionDeny, v.Action)
	assert.Equal(t, "device_player_limit", v.Rule)
}

func TestCheckRedeemPassesCleanRequest(t *testing.T) {
	v := risk.CheckRedeem(risk.RedeemInput{
		DevicePlayerCount: 1,
		IPRedeemToday:     2,
		TGUserID:          123456789,
		ClickToRedeem:     45 * time.Second,
	})

	assert.Equal(t, risk.ActionPass, v.Action)
}

// ClickToRedeem 为零表示信号缺失（比如老数据），不应据此判定异常。
func TestCheckRedeemIgnoresMissingLatency(t *testing.T) {
	v := risk.CheckRedeem(risk.RedeemInput{TGUserID: 123456789, ClickToRedeem: 0})

	assert.Equal(t, risk.ActionPass, v.Action)
}
