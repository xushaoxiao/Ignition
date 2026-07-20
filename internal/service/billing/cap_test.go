package billing_test

import (
	"testing"
	"time"

	"github.com/shaoxiaoxu/linksprout/internal/models"
	"github.com/shaoxiaoxu/linksprout/internal/service/billing"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func evs(amounts ...int64) []models.BillableEvent {
	out := make([]models.BillableEvent, len(amounts))
	for i, a := range amounts {
		out[i] = models.BillableEvent{ID: int64(i + 1), AmountCents: a, Currency: "USD"}
	}
	return out
}

func TestApplyCapSplitsAtLimit(t *testing.T) {
	res := billing.ApplyCap(evs(200, 200, 200, 200), 500)

	assert.Len(t, res.Billable, 2, "只有前两笔在额度内")
	assert.EqualValues(t, 400, res.BilledCents)
	assert.Len(t, res.OverCap, 2)
	assert.EqualValues(t, 400, res.WaivedCents)
}

// 超封顶的事件不是被丢弃，而是标记为免费。它们照常归因、照常给 KOL 记功，
// 只是不进 invoice —— 这既是更好的客户体验，也是最自然的升档话术。
func TestOverCapEventsAreMarkedNotDropped(t *testing.T) {
	res := billing.ApplyCap(evs(300, 300), 300)

	require.Len(t, res.OverCap, 1)
	assert.True(t, res.OverCap[0].OverCap, "超封顶事件必须被标记")
	assert.EqualValues(t, 2, res.OverCap[0].ID, "被免费的应是后发生的那笔")
	assert.Len(t, res.Billable, 1)
	assert.False(t, res.Billable[0].OverCap)
}

func TestApplyCapZeroMeansUnlimited(t *testing.T) {
	res := billing.ApplyCap(evs(200, 200, 200), 0)

	assert.Len(t, res.Billable, 3)
	assert.Empty(t, res.OverCap)
	assert.EqualValues(t, 600, res.BilledCents)
	assert.Zero(t, res.WaivedCents)
}

// 一笔大额事件若会击穿封顶，整笔不计费，而不是部分计费。
// 部分计费会让客户看到"半笔转化"，对不上任何东西。
func TestApplyCapDoesNotSplitSingleEvent(t *testing.T) {
	res := billing.ApplyCap(evs(100, 900), 500)

	assert.Len(t, res.Billable, 1)
	assert.EqualValues(t, 100, res.BilledCents)
	assert.EqualValues(t, 900, res.WaivedCents)
}

func TestTransitionRejectsIllegal(t *testing.T) {
	ev := models.BillableEvent{Status: models.StatusBilled}
	err := billing.Transition(&ev, models.StatusCleared, "", time.Now())

	require.Error(t, err)
	assert.Equal(t, models.StatusBilled, ev.Status, "非法迁移不得改动状态")
}

func TestTransitionStampsTimestamps(t *testing.T) {
	now := time.Date(2026, 7, 20, 10, 0, 0, 0, time.UTC)
	ev := models.BillableEvent{Status: models.StatusPending}

	require.NoError(t, billing.Transition(&ev, models.StatusCleared, "hold_elapsed", now))
	require.NotNil(t, ev.ClearedAt)
	assert.Equal(t, now, *ev.ClearedAt)
	assert.Equal(t, "hold_elapsed", ev.StatusReason)

	require.NoError(t, billing.Transition(&ev, models.StatusBilled, "invoiced", now))
	require.NotNil(t, ev.BilledAt)
}

func TestReadyToClearRespectsHoldPeriod(t *testing.T) {
	now := time.Date(2026, 7, 20, 10, 0, 0, 0, time.UTC)

	inHold := models.BillableEvent{Status: models.StatusPending, HoldUntil: now.Add(time.Hour)}
	assert.False(t, billing.ReadyToClear(inHold, now))

	elapsed := models.BillableEvent{Status: models.StatusPending, HoldUntil: now.Add(-time.Hour)}
	assert.True(t, billing.ReadyToClear(elapsed, now))
}

// 风控暂缓的事件不会因为时间流逝自动放行 —— 暂缓的语义是"在人看过之前
// 不收这笔钱"，自动过期会让暂缓形同虚设。
func TestHeldEventsNeverAutoClear(t *testing.T) {
	now := time.Date(2026, 7, 20, 10, 0, 0, 0, time.UTC)
	held := models.BillableEvent{Status: models.StatusHeld, HoldUntil: now.Add(-100 * time.Hour)}

	assert.False(t, billing.ReadyToClear(held, now))
}
