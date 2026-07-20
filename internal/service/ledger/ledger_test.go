package ledger_test

import (
	"testing"

	"github.com/shaoxiaoxu/linksprout/internal/models"
	"github.com/shaoxiaoxu/linksprout/internal/service/ledger"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestChargeCPABalances(t *testing.T) {
	ev := models.BillableEvent{ID: 42, AmountCents: 200, Currency: "USD"}
	txn := ledger.ChargeCPA(1, ev)

	require.NoError(t, txn.Validate())
	bal := ledger.Balance(txn.Entries)
	assert.EqualValues(t, 200, bal[models.AccountTenantReceivable], "客户应付增加")
	assert.EqualValues(t, -200, bal[models.AccountPlatformRevenue], "收入贷方增加")
}

// 冲正后净额必须归零，且原分录仍然存在 —— 这是可审计性的来源。
func TestReverseNetsToZero(t *testing.T) {
	orig := ledger.ChargeCPA(1, models.BillableEvent{ID: 42, AmountCents: 200, Currency: "USD"})
	rev := ledger.Reverse(orig, "reversal", 42)

	require.NoError(t, rev.Validate())
	assert.NotEqual(t, orig.ID, rev.ID, "冲正是一笔新交易，不是修改原交易")

	combined := append(append([]ledger.Entry{}, orig.Entries...), rev.Entries...)
	for account, amount := range ledger.Balance(combined) {
		assert.Zero(t, amount, "科目 %s 冲正后应归零", account)
	}
}

func TestValidateRejectsUnbalanced(t *testing.T) {
	txn := ledger.Txn{Entries: []ledger.Entry{
		{models.AccountTenantReceivable, models.Debit, 200, "USD"},
		{models.AccountPlatformRevenue, models.Credit, 100, "USD"},
	}}
	assert.ErrorIs(t, txn.Validate(), ledger.ErrUnbalanced)
}

func TestValidateRejectsMixedCurrency(t *testing.T) {
	txn := ledger.Txn{Entries: []ledger.Entry{
		{models.AccountTenantReceivable, models.Debit, 200, "USD"},
		{models.AccountPlatformRevenue, models.Credit, 200, "EUR"},
	}}
	assert.ErrorIs(t, txn.Validate(), ledger.ErrMixedCurrency)
}

// 金额必须为正，方向由 Direction 表达。允许负数会让同一笔账有两种写法，
// 对账时无法判断哪种是对的。
func TestValidateRejectsNonPositive(t *testing.T) {
	txn := ledger.Txn{Entries: []ledger.Entry{
		{models.AccountTenantReceivable, models.Debit, -200, "USD"},
		{models.AccountPlatformRevenue, models.Credit, -200, "USD"},
	}}
	assert.ErrorIs(t, txn.Validate(), ledger.ErrNonPositive)
}

func TestValidateRejectsEmpty(t *testing.T) {
	assert.ErrorIs(t, ledger.Txn{}.Validate(), ledger.ErrEmptyEntries)
}
