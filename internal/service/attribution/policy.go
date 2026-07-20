package attribution

import (
	"fmt"
	"time"

	"github.com/shaoxiaoxu/linksprout/internal/models"
)

// Policy 归因规则的一个版本。
//
// 这张表就是产品本身：客户按它验算，KOL 按它申诉。规则变更必须发新版本号并
// 提前通知，绝不静默修改（设计约束 C2）。每条 Attribution 记录都会存下当时的
// PolicyVersion，使得任何一笔历史账单都能被精确复算。
type Policy struct {
	Version string

	// ClickWindow 点击归因窗口：超过则该点击失效。
	ClickWindow time.Duration
	// ClaimCodeTTL 领奖码有效期。
	ClaimCodeTTL time.Duration
	// LockPeriod 归因锁定期：期内该 Player 的转化都算原 KOL。
	LockPeriod time.Duration
	// HoldPeriods 各事件类型的冷静期，期内可冲正。
	HoldPeriods map[string]time.Duration
}

// PolicyV1 当前生效的归因规则。对应 docs/attribution-policy-v1.md。
var PolicyV1 = Policy{
	Version:      "v1",
	ClickWindow:  7 * 24 * time.Hour,
	ClaimCodeTTL: 24 * time.Hour,
	LockPeriod:   90 * 24 * time.Hour,
	HoldPeriods: map[string]time.Duration{
		// activation 由我方核销确认，7 天足够覆盖异常发现窗口
		models.EventActivation: 7 * 24 * time.Hour,
		// 未来的 GMV 分成需覆盖 App Store 退款窗口，故取 35 天
		models.EventIAPPurchase: 35 * 24 * time.Hour,
	},
}

var policies = map[string]Policy{PolicyV1.Version: PolicyV1}

// PolicyByVersion 按版本号取回归因规则，用于申诉复算历史记录。
func PolicyByVersion(v string) (Policy, error) {
	p, ok := policies[v]
	if !ok {
		return Policy{}, fmt.Errorf("attribution: unknown policy version %q", v)
	}
	return p, nil
}

// HoldPeriod 返回某事件类型的冷静期。未配置的类型回退到 7 天，
// 宁可多押一会儿也不要立刻计费。
func (p Policy) HoldPeriod(eventType string) time.Duration {
	if d, ok := p.HoldPeriods[eventType]; ok {
		return d
	}
	return 7 * 24 * time.Hour
}
