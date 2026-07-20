package models_test

import (
	"testing"

	"github.com/shaoxiaoxu/linksprout/internal/models"
	"github.com/stretchr/testify/assert"
)

// 这条测试守护整个商业模型的地基：中层效果分成只对确定性归因收费。
// 如果有人不小心把 probabilistic 改成可计费，账单立刻失去公信力。
func TestOnlyDeterministicMethodsAreBillable(t *testing.T) {
	billable := []models.AttributionMethod{
		models.MethodDeterministicCode,
		models.MethodInstallReferrer,
		models.MethodUniversalLink,
	}
	notBillable := []models.AttributionMethod{
		models.MethodClipboardMatch,
		models.MethodProbabilistic,
	}

	for _, m := range billable {
		assert.True(t, m.IsBillable(), "%s 应可计费", m)
		assert.EqualValues(t, 100, m.Confidence(), "%s 置信度应为 100", m)
	}
	for _, m := range notBillable {
		assert.False(t, m.IsBillable(), "%s 不应计费", m)
		assert.Less(t, m.Confidence(), int16(100), "%s 置信度应低于 100", m)
	}
}

// 未知归因方式必须 fail-closed：不可计费、置信度 0。
func TestUnknownMethodFailsClosed(t *testing.T) {
	var m models.AttributionMethod = "some_new_method"
	assert.False(t, m.Valid())
	assert.False(t, m.IsBillable())
	assert.Zero(t, m.Confidence())
}

func TestBillableStateMachine(t *testing.T) {
	legal := []struct{ from, to models.BillableStatus }{
		{models.StatusPending, models.StatusCleared},
		{models.StatusPending, models.StatusHeld},
		{models.StatusPending, models.StatusRejected},
		{models.StatusHeld, models.StatusCleared},
		{models.StatusHeld, models.StatusRejected},
		{models.StatusCleared, models.StatusBilled},
		{models.StatusCleared, models.StatusReversed},
		// 已开票仍可冲正 —— 走下个账期的 credit，不追溯改已出账单
		{models.StatusBilled, models.StatusReversed},
	}
	for _, c := range legal {
		assert.True(t, models.CanTransition(c.from, c.to), "%s -> %s 应合法", c.from, c.to)
	}

	illegal := []struct{ from, to models.BillableStatus }{
		{models.StatusBilled, models.StatusCleared},   // 不可回退重新放行
		{models.StatusReversed, models.StatusCleared}, // 冲正是终态
		{models.StatusRejected, models.StatusCleared}, // 驳回是终态
		{models.StatusPending, models.StatusBilled},   // 必须先过冷静期
		{models.StatusHeld, models.StatusBilled},      // 暂缓的不能直接开票
	}
	for _, c := range illegal {
		assert.False(t, models.CanTransition(c.from, c.to), "%s -> %s 应非法", c.from, c.to)
	}
}
