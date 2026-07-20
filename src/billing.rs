//! 可计费事件的推进、封顶与账单结算。

use chrono::{DateTime, Utc};

use crate::models::{BillableEvent, BillableStatus, Cents, IllegalTransition};

/// 封顶计算的结果。
#[derive(Debug, Default)]
pub struct CapResult {
    /// 计入账单的事件
    pub billable: Vec<BillableEvent>,
    /// 超出封顶的事件：照常归因、照常给 KOL 记功，只是不收费
    pub over_cap: Vec<BillableEvent>,
    /// 本期实际计费金额
    pub billed: Cents,
    /// 因封顶而免收的金额，用于在看板上展示「本月免费送了你多少」
    pub waived: Cents,
}

/// 对一个账期内已放行的事件应用月度封顶。
///
/// 封顶是给客户的确定性承诺，实现上有两个刻意的选择：
///
/// 1. 权威计算发生在月末结算（这里），而不是写入时的 Redis 计数器。
///    Redis 计数在并发、重启、冲正回退下都不可靠，只适合做实时的软提示。
///    账单必须由 Postgres 里的事实重新算一遍。
///
/// 2. 超出封顶的转化不是「拒绝」而是「免费」。事件照常记录、照常归因、
///    KOL 照常记功，只是不进 invoice。这比「超出后停服」体验好得多，
///    而且是最自然的升档话术。
///
/// `events` 必须按 `cleared_at` 升序传入 —— 先发生的转化先占用额度，
/// 这是唯一对客户可解释的顺序。`cap` 为 `None` 表示无封顶。
pub fn apply_cap(events: Vec<BillableEvent>, cap: Option<Cents>) -> CapResult {
    let mut res = CapResult::default();
    for mut ev in events {
        let would_exceed = cap.is_some_and(|c| res.billed + ev.amount_cents > c);
        if would_exceed {
            ev.over_cap = true;
            res.waived += ev.amount_cents;
            res.over_cap.push(ev);
        } else {
            ev.over_cap = false;
            res.billed += ev.amount_cents;
            res.billable.push(ev);
        }
    }
    res
}

/// 校验并推进事件状态。
///
/// 所有状态变更都必须走这里，不允许直接给 `status` 赋值 —— 状态机是收入
/// 正确性的核心约束，散落的赋值会让「已开票的事件被重新 clear」这类问题
/// 无法被静态发现。
pub fn transition(
    ev: &mut BillableEvent,
    to: BillableStatus,
    reason: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(), IllegalTransition> {
    if !ev.status.can_transition_to(to) {
        return Err(IllegalTransition {
            from: ev.status,
            to,
        });
    }
    ev.status = to;
    ev.status_reason = reason.map(str::to_string);
    match to {
        BillableStatus::Cleared => ev.cleared_at = Some(now),
        BillableStatus::Billed => ev.billed_at = Some(now),
        _ => {}
    }
    Ok(())
}

/// 事件是否已过冷静期、可以放行。
///
/// `Held` 状态需要人工复核，不会因为时间流逝自动放行 —— 风控暂缓的语义是
/// 「在人看过之前不收这笔钱」，自动过期会让暂缓形同虚设。
pub fn ready_to_clear(ev: &BillableEvent, now: DateTime<Utc>) -> bool {
    ev.status == BillableStatus::Pending && now >= ev.hold_until
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::event_type;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 20, hour, 0, 0).unwrap()
    }

    fn events(amounts: &[i64]) -> Vec<BillableEvent> {
        amounts
            .iter()
            .enumerate()
            .map(|(i, &a)| BillableEvent {
                id: i as i64 + 1,
                tenant_id: 1,
                attribution_id: 1,
                event_type: event_type::ACTIVATION.into(),
                external_id: format!("claim:{}", i + 1),
                status: BillableStatus::Cleared,
                amount_cents: Cents(a),
                currency: "USD".into(),
                over_cap: false,
                occurred_at: at(10),
                received_at: at(10),
                hold_until: at(10),
                cleared_at: None,
                billed_at: None,
                invoice_id: None,
                status_reason: None,
            })
            .collect()
    }

    #[test]
    fn apply_cap_splits_at_limit() {
        let res = apply_cap(events(&[200, 200, 200, 200]), Some(Cents(500)));

        assert_eq!(res.billable.len(), 2, "只有前两笔在额度内");
        assert_eq!(res.billed, Cents(400));
        assert_eq!(res.over_cap.len(), 2);
        assert_eq!(res.waived, Cents(400));
    }

    /// 超封顶的事件不是被丢弃，而是标记为免费。它们照常归因、照常给 KOL
    /// 记功，只是不进 invoice。
    #[test]
    fn over_cap_events_are_marked_not_dropped() {
        let res = apply_cap(events(&[300, 300]), Some(Cents(300)));

        assert_eq!(res.over_cap.len(), 1);
        assert!(res.over_cap[0].over_cap, "超封顶事件必须被标记");
        assert_eq!(res.over_cap[0].id, 2, "被免费的应是后发生的那笔");
        assert_eq!(res.billable.len(), 1);
        assert!(!res.billable[0].over_cap);
    }

    #[test]
    fn no_cap_means_unlimited() {
        let res = apply_cap(events(&[200, 200, 200]), None);

        assert_eq!(res.billable.len(), 3);
        assert!(res.over_cap.is_empty());
        assert_eq!(res.billed, Cents(600));
        assert_eq!(res.waived, Cents::ZERO);
    }

    /// 一笔大额事件若会击穿封顶，整笔不计费，而不是部分计费。
    /// 部分计费会让客户看到「半笔转化」，对不上任何东西。
    #[test]
    fn does_not_split_a_single_event() {
        let res = apply_cap(events(&[100, 900]), Some(Cents(500)));

        assert_eq!(res.billable.len(), 1);
        assert_eq!(res.billed, Cents(100));
        assert_eq!(res.waived, Cents(900));
    }

    #[test]
    fn transition_rejects_illegal() {
        let mut ev = events(&[200]).pop().unwrap();
        ev.status = BillableStatus::Billed;

        let err = transition(&mut ev, BillableStatus::Cleared, None, at(10)).unwrap_err();

        assert_eq!(err.from, BillableStatus::Billed);
        assert_eq!(ev.status, BillableStatus::Billed, "非法迁移不得改动状态");
    }

    #[test]
    fn transition_stamps_timestamps() {
        let mut ev = events(&[200]).pop().unwrap();
        ev.status = BillableStatus::Pending;

        transition(
            &mut ev,
            BillableStatus::Cleared,
            Some("hold_elapsed"),
            at(10),
        )
        .unwrap();
        assert_eq!(ev.cleared_at, Some(at(10)));
        assert_eq!(ev.status_reason.as_deref(), Some("hold_elapsed"));

        transition(&mut ev, BillableStatus::Billed, Some("invoiced"), at(11)).unwrap();
        assert_eq!(ev.billed_at, Some(at(11)));
    }

    #[test]
    fn ready_to_clear_respects_hold_period() {
        let mut ev = events(&[200]).pop().unwrap();
        ev.status = BillableStatus::Pending;

        ev.hold_until = at(12);
        assert!(!ready_to_clear(&ev, at(10)), "冷静期内不放行");

        ev.hold_until = at(9);
        assert!(ready_to_clear(&ev, at(10)), "冷静期已过应放行");
    }

    /// 风控暂缓的事件不会因为时间流逝自动放行 —— 暂缓的语义是「在人看过
    /// 之前不收这笔钱」，自动过期会让暂缓形同虚设。
    #[test]
    fn held_events_never_auto_clear() {
        let mut ev = events(&[200]).pop().unwrap();
        ev.status = BillableStatus::Held;
        ev.hold_until = at(1);

        assert!(!ready_to_clear(&ev, at(23)));
    }
}
