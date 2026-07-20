// Package models 定义领域类型与跨模块共享的枚举。
//
// 这里承载设计文档 §3 的领域模型，以及两条不可违反的规则：
//   - 归因方式与"是否可计费"的映射（约束 C1）
//   - 可计费事件的状态机（约束 C3）
package models

import (
	"encoding/json"
	"fmt"
	"time"
)

// ---------------------------------------------------------------- 归因

// AttributionMethod 归因方式。取值与数据库 attribution_method 枚举一一对应。
type AttributionMethod string

const (
	// MethodDeterministicCode 领奖码核销 —— iOS 上唯一可计费的路径
	MethodDeterministicCode AttributionMethod = "deterministic_code"
	// MethodInstallReferrer Android Play Install Referrer，用户零操作
	MethodInstallReferrer AttributionMethod = "install_referrer"
	// MethodUniversalLink 已安装用户直接唤起，参数在 URL 里
	MethodUniversalLink AttributionMethod = "universal_link"
	// MethodClipboardMatch 剪贴板匹配 —— 可提升看板转化率，但不进账单
	MethodClipboardMatch AttributionMethod = "clipboard_match"
	// MethodProbabilistic 指纹/时间窗匹配 —— iOS 17+ 精度已崩塌，仅统计
	MethodProbabilistic AttributionMethod = "probabilistic"
)

// methodProfile 归因方式的固有属性。
//
// billable 的取值是整个商业模型的地基：中层效果分成只对确定性归因收费，
// 概率归因的转化可以进看板，但绝不能进账单（设计约束 C1）。
var methodProfile = map[AttributionMethod]struct {
	confidence int16
	billable   bool
}{
	MethodDeterministicCode: {100, true},
	MethodInstallReferrer:   {100, true},
	MethodUniversalLink:     {100, true},
	MethodClipboardMatch:    {60, false},
	MethodProbabilistic:     {30, false},
}

// Confidence 返回该归因方式的置信度。未知方式返回 0。
func (m AttributionMethod) Confidence() int16 { return methodProfile[m].confidence }

// IsBillable 返回该归因方式是否可计费。未知方式一律不可计费（fail-closed）。
func (m AttributionMethod) IsBillable() bool { return methodProfile[m].billable }

// Valid 报告 m 是否为已知的归因方式。
func (m AttributionMethod) Valid() bool {
	_, ok := methodProfile[m]
	return ok
}

// Attribution 归因记录，系统的信任基石。
//
// Evidence 存判定当时的完整输入快照，是 KOL 申诉时唯一的证据来源，只增不改。
// PolicyVersion 对应一份对客户公开的规则文档，规则变更必须发新版本号。
type Attribution struct {
	ID            int64
	TenantID      int64
	PlayerID      int64
	KOLID         int64
	CampaignID    int64
	LinkID        int64
	Method        AttributionMethod
	Confidence    int16
	IsBillable    bool
	PolicyVersion string
	TouchAt       time.Time
	AttributedAt  time.Time
	LockedUntil   time.Time
	Evidence      json.RawMessage
}

// ---------------------------------------------------------------- 计费

// BillableStatus 可计费事件的状态。
type BillableStatus string

const (
	StatusPending  BillableStatus = "pending"  // 已接收，在 hold 冷静期内
	StatusHeld     BillableStatus = "held"     // 风控暂缓，等待人工复核
	StatusCleared  BillableStatus = "cleared"  // 已放行，可计入账单
	StatusBilled   BillableStatus = "billed"   // 已开票
	StatusReversed BillableStatus = "reversed" // 已冲正（退款 / 事后判定作弊）
	StatusRejected BillableStatus = "rejected" // 判定无效，不计费
)

// allowedTransitions 可计费事件的状态机（设计文档 §3）。
//
//	              ┌──────────► rejected
//	              │
//	pending ──► cleared ──► billed
//	    │         ▲            │
//	    ▼         │            ▼
//	  held ───────┘         reversed
//
// billed 之后仍可 reversed —— 冲正走下个账期的 credit，不追溯改已出账单。
var allowedTransitions = map[BillableStatus][]BillableStatus{
	StatusPending:  {StatusCleared, StatusHeld, StatusRejected},
	StatusHeld:     {StatusCleared, StatusRejected},
	StatusCleared:  {StatusBilled, StatusReversed, StatusRejected},
	StatusBilled:   {StatusReversed},
	StatusReversed: {},
	StatusRejected: {},
}

// CanTransition 报告状态能否从 from 迁移到 to。
func CanTransition(from, to BillableStatus) bool {
	for _, s := range allowedTransitions[from] {
		if s == to {
			return true
		}
	}
	return false
}

// ErrIllegalTransition 表示一次非法的状态迁移。
type ErrIllegalTransition struct{ From, To BillableStatus }

func (e ErrIllegalTransition) Error() string {
	return fmt.Sprintf("billable event: illegal transition %s -> %s", e.From, e.To)
}

// BillableEvent 可计费事件，收入的原子。
//
// 只有 Attribution.IsBillable 为 true 的转化才会产生 BillableEvent；
// 不可计费的转化只进分析流（ClickHouse），不进这张表。
type BillableEvent struct {
	ID            int64
	TenantID      int64
	AttributionID int64
	EventType     string // activation | iap_purchase
	ExternalID    string // 主 App 侧唯一 ID，幂等键
	Status        BillableStatus
	AmountCents   int64
	Currency      string
	OverCap       bool
	OccurredAt    time.Time
	ReceivedAt    time.Time
	HoldUntil     time.Time
	ClearedAt     *time.Time
	BilledAt      *time.Time
	InvoiceID     *int64
	StatusReason  string
}

// 事件类型。
const (
	EventActivation  = "activation"
	EventIAPPurchase = "iap_purchase"
)

// ---------------------------------------------------------------- 账本

// Account 账本科目。
type Account string

const (
	AccountTenantReceivable Account = "tenant_receivable" // 客户应付我方
	AccountPlatformRevenue  Account = "platform_revenue"  // 平台收入
	AccountKOLPayable       Account = "kol_payable"       // 我方应付 KOL
	AccountReversalClearing Account = "reversal_clearing" // 冲正过渡
)

// Direction 借贷方向。
type Direction string

const (
	Debit  Direction = "D"
	Credit Direction = "C"
)

// ---------------------------------------------------------------- 其它

// ClaimStatus 领奖码状态。
type ClaimStatus string

const (
	ClaimIssued   ClaimStatus = "issued"
	ClaimRedeemed ClaimStatus = "redeemed"
	ClaimExpired  ClaimStatus = "expired"
	ClaimVoided   ClaimStatus = "voided"
)

// Player 终端用户。TG 侧身份来自 initData（可信），App 侧身份在核销时绑定。
type Player struct {
	ID          int64
	TenantID    int64
	TGUserID    int64
	AppUserID   *string
	DeviceIDs   []string
	TGIsPremium bool
	TGUsername  string
	FirstSeenAt time.Time
}
