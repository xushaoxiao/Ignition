// Package billing 实现可计费事件的推进、封顶与账单结算。
package billing

import (
	"time"

	"github.com/shaoxiaoxu/linksprout/internal/models"
)

// CapResult 封顶计算的结果。
type CapResult struct {
	// Billable 计入账单的事件
	Billable []models.BillableEvent
	// OverCap 超出封顶的事件：照常归因、照常给 KOL 记功，只是不收费
	OverCap []models.BillableEvent
	// BilledCents 本期实际计费金额
	BilledCents int64
	// WaivedCents 因封顶而免收的金额，用于在看板上展示"本月免费送了你多少"
	WaivedCents int64
}

// ApplyCap 对一个账期内已放行的事件应用月度封顶。
//
// 封顶是给客户的确定性承诺，实现上有两个刻意的选择：
//
//  1. 权威计算发生在月末结算（这里），而不是写入时的 Redis 计数器。
//     Redis 计数在并发、重启、冲正回退下都不可靠，只适合做实时的软提示。
//     账单必须由 Postgres 里的事实重新算一遍。
//
//  2. 超出封顶的转化不是"拒绝"而是"免费"。事件照常记录、照常归因、
//     KOL 照常记功，只是不进 invoice。这比"超出后停服"体验好得多，
//     而且是最自然的升档话术。
//
// events 必须按 ClearedAt 升序传入 —— 先发生的转化先占用额度，这是唯一
// 对客户可解释的顺序。capCents <= 0 表示无封顶。
func ApplyCap(events []models.BillableEvent, capCents int64) CapResult {
	res := CapResult{
		Billable: make([]models.BillableEvent, 0, len(events)),
		OverCap:  make([]models.BillableEvent, 0),
	}
	for _, ev := range events {
		if capCents > 0 && res.BilledCents+ev.AmountCents > capCents {
			ev.OverCap = true
			res.OverCap = append(res.OverCap, ev)
			res.WaivedCents += ev.AmountCents
			continue
		}
		ev.OverCap = false
		res.Billable = append(res.Billable, ev)
		res.BilledCents += ev.AmountCents
	}
	return res
}

// Transition 校验并推进事件状态。
//
// 所有状态变更都必须走这里，不允许直接赋值 —— 状态机是收入正确性的核心
// 约束，散落的赋值会让"已开票的事件被重新 clear"这类问题无法被静态发现。
func Transition(ev *models.BillableEvent, to models.BillableStatus, reason string, now time.Time) error {
	if !models.CanTransition(ev.Status, to) {
		return models.ErrIllegalTransition{From: ev.Status, To: to}
	}
	ev.Status = to
	ev.StatusReason = reason
	switch to {
	case models.StatusCleared:
		t := now
		ev.ClearedAt = &t
	case models.StatusBilled:
		t := now
		ev.BilledAt = &t
	}
	return nil
}

// ReadyToClear 报告事件是否已过冷静期、可以放行。
//
// held 状态需要人工复核，不会因为时间流逝自动放行 —— 风控暂缓的语义是
// "在人看过之前不收这笔钱"，自动过期会让暂缓形同虚设。
func ReadyToClear(ev models.BillableEvent, now time.Time) bool {
	return ev.Status == models.StatusPending && !now.Before(ev.HoldUntil)
}
