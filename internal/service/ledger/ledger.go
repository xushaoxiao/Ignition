// Package ledger 实现复式记账。
//
// 设计约束 C3：账本只可追加，不可修改。退款、拒付、事后判定作弊都通过写一笔
// 反向分录来表达，而不是 UPDATE 原记录。这样任何一个时点的余额都可以由分录
// 重放得出，客户申诉时能拿出完整的、未被改动过的证据链。
package ledger

import (
	"errors"
	"fmt"

	"github.com/google/uuid"
	"github.com/shaoxiaoxu/linksprout/internal/models"
)

var (
	ErrUnbalanced   = errors.New("ledger: 借贷不平")
	ErrEmptyEntries = errors.New("ledger: 交易不含任何分录")
	ErrMixedCurrency = errors.New("ledger: 单笔交易混用币种")
	ErrNonPositive  = errors.New("ledger: 分录金额必须为正")
)

// Entry 一条分录。金额恒为正，方向由 Direction 表达。
type Entry struct {
	Account   models.Account
	Direction models.Direction
	Amount    int64 // 最小货币单位（cent），恒 > 0
	Currency  string
}

// Txn 一笔交易，由若干条借贷平衡的分录组成。
type Txn struct {
	ID       uuid.UUID
	TenantID int64
	RefType  string
	RefID    int64
	Entries  []Entry
}

// Validate 校验交易的完整性：非空、金额为正、币种一致、借贷平衡。
//
// 这是账本的守门人 —— 一旦不平衡的分录写进库，后续所有对账都会失效，
// 而由于账本不可修改，修复只能靠更多的补偿分录。所以宁可在这里拒绝。
func (t Txn) Validate() error {
	if len(t.Entries) == 0 {
		return ErrEmptyEntries
	}
	var debit, credit int64
	currency := t.Entries[0].Currency
	for _, e := range t.Entries {
		if e.Amount <= 0 {
			return fmt.Errorf("%w: account=%s amount=%d", ErrNonPositive, e.Account, e.Amount)
		}
		if e.Currency != currency {
			return fmt.Errorf("%w: %s vs %s", ErrMixedCurrency, currency, e.Currency)
		}
		switch e.Direction {
		case models.Debit:
			debit += e.Amount
		case models.Credit:
			credit += e.Amount
		default:
			return fmt.Errorf("ledger: 非法方向 %q", e.Direction)
		}
	}
	if debit != credit {
		return fmt.Errorf("%w: debit=%d credit=%d", ErrUnbalanced, debit, credit)
	}
	return nil
}

// ChargeCPA 构造一笔 CPA 计费的交易。
//
//	D  tenant_receivable   客户应付我方
//	C  platform_revenue    确认收入
func ChargeCPA(tenantID int64, ev models.BillableEvent) Txn {
	return Txn{
		ID:       uuid.New(),
		TenantID: tenantID,
		RefType:  "billable_event",
		RefID:    ev.ID,
		Entries: []Entry{
			{models.AccountTenantReceivable, models.Debit, ev.AmountCents, ev.Currency},
			{models.AccountPlatformRevenue, models.Credit, ev.AmountCents, ev.Currency},
		},
	}
}

// ChargePlatformFee 构造一笔平台订阅费的交易。
func ChargePlatformFee(tenantID int64, subscriptionID, amount int64, currency string) Txn {
	return Txn{
		ID:       uuid.New(),
		TenantID: tenantID,
		RefType:  "subscription",
		RefID:    subscriptionID,
		Entries: []Entry{
			{models.AccountTenantReceivable, models.Debit, amount, currency},
			{models.AccountPlatformRevenue, models.Credit, amount, currency},
		},
	}
}

// Reverse 构造一笔冲正交易：方向与原交易完全相反。
//
// 刻意不去修改原分录，也不去"删掉"它。原交易与冲正交易在账本上并存，
// 净额为零，但两者都留有痕迹 —— 这正是可审计性的来源。
func Reverse(orig Txn, refType string, refID int64) Txn {
	rev := Txn{
		ID:       uuid.New(),
		TenantID: orig.TenantID,
		RefType:  refType,
		RefID:    refID,
		Entries:  make([]Entry, 0, len(orig.Entries)),
	}
	for _, e := range orig.Entries {
		d := models.Credit
		if e.Direction == models.Credit {
			d = models.Debit
		}
		rev.Entries = append(rev.Entries, Entry{e.Account, d, e.Amount, e.Currency})
	}
	return rev
}

// Balance 按科目汇总一组分录的净额（借为正，贷为负）。
// 用于日常的不变量校验与对账。
func Balance(entries []Entry) map[models.Account]int64 {
	out := make(map[models.Account]int64)
	for _, e := range entries {
		if e.Direction == models.Debit {
			out[e.Account] += e.Amount
		} else {
			out[e.Account] -= e.Amount
		}
	}
	return out
}
