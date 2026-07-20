// Package risk 实现风控的 L1 硬约束与 L2 信号定义。
//
// 风控在这个系统里有双重身份：既保护奖励成本，也保护计费准确性。后者更重要
// —— 一条被判定为作弊的转化，如果已经收了客户的钱，损害的是信任而不只是钱。
package risk

import (
	"time"
)

// Action L1 规则的处置动作。
type Action string

const (
	// ActionPass 放行
	ActionPass Action = "pass"
	// ActionHold 标记暂缓：事件照常记录，但不进账单，等人工复核
	ActionHold Action = "hold"
	// ActionDeny 直接拒绝
	ActionDeny Action = "deny"
)

// Verdict 一次风控判定。
type Verdict struct {
	Action Action
	Rule   string
	Detail map[string]any
}

func pass() Verdict { return Verdict{Action: ActionPass, Rule: "none"} }

// PlayInput 抽奖前的 L1 检查输入。
type PlayInput struct {
	TodayPlayCount  int
	DailyPlayLimit  int
	PlayerDeviceCnt int
}

// CheckPlay 抽奖前的硬约束。
//
// 抽奖直接消耗奖池成本，且重来一次对真实用户没有损失，所以这里可以直接拒绝。
func CheckPlay(in PlayInput) Verdict {
	if in.DailyPlayLimit > 0 && in.TodayPlayCount >= in.DailyPlayLimit {
		return Verdict{ActionDeny, "daily_play_limit", map[string]any{
			"count": in.TodayPlayCount, "limit": in.DailyPlayLimit,
		}}
	}
	return pass()
}

// RedeemInput 核销时的 L1 检查输入。
type RedeemInput struct {
	// DevicePlayerCount 该 device_id 已绑定的 Player 数
	DevicePlayerCount int
	// IPRedeemToday 该 IP 今日核销数
	IPRedeemToday int
	// TGUserID 用于粗判账号年龄：TG 的 user_id 大体单调递增
	TGUserID int64
	// ClickToRedeem 从点击到核销的总耗时
	ClickToRedeem time.Duration
}

// 阈值。刻意设为变量而非常量：这些数字需要按真实数据调，
// 而不是靠改代码发版。生产环境应从配置或 campaign 上读取。
var (
	MaxPlayersPerDevice = 3
	MaxRedeemPerIPDay   = 10
	// NewAccountThreshold 超过此值的 tg_user_id 视为新注册账号。
	// TG user_id 大体随注册时间递增，这是个粗糙但零成本的信号。
	// 需要定期按实际数据校准。
	NewAccountThreshold int64 = 7_500_000_000
	// MinClickToRedeem 低于此耗时视为脚本特征。
	MinClickToRedeem = 1500 * time.Millisecond
)

// CheckRedeem 核销时的硬约束。
//
// 关键取舍：这里尽量只 hold 不 deny。
//
// 核销是用户旅程的终点，误杀一个真实用户 = 他领不到奖 + 对客户 App 的第一印象
// 是"这破活动是骗人的"，这个损失不可挽回。而放过一个刷子只是暂时多算一笔，
// 冷静期内可以冲正、可以人工驳回，钱还没真正付出去。
//
// 唯一直接拒绝的是设备维度 —— 一台设备绑定过多账号是明确的养号特征，
// 且真实用户几乎不可能触发。
func CheckRedeem(in RedeemInput) Verdict {
	if in.DevicePlayerCount >= MaxPlayersPerDevice {
		return Verdict{ActionDeny, "device_player_limit", map[string]any{
			"count": in.DevicePlayerCount, "limit": MaxPlayersPerDevice,
		}}
	}
	if in.IPRedeemToday >= MaxRedeemPerIPDay {
		return Verdict{ActionHold, "ip_redeem_rate", map[string]any{
			"count": in.IPRedeemToday, "limit": MaxRedeemPerIPDay,
		}}
	}
	if in.ClickToRedeem > 0 && in.ClickToRedeem < MinClickToRedeem {
		return Verdict{ActionHold, "too_fast", map[string]any{
			"elapsed_ms": in.ClickToRedeem.Milliseconds(),
		}}
	}
	if in.TGUserID > NewAccountThreshold {
		return Verdict{ActionHold, "new_tg_account", map[string]any{
			"tg_user_id": in.TGUserID,
		}}
	}
	return pass()
}
