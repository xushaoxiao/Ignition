//! Billable-event progression, caps, and settlement helpers.

use chrono::{DateTime, Utc};

use crate::models::{BillableEvent, BillableStatus, Cents, IllegalTransition};

/// Result of applying a monthly cap.
#[derive(Debug, Default)]
pub struct CapResult {
    /// Events included on the invoice
    pub billable: Vec<BillableEvent>,
    /// Over-cap events: still attributed and credited to the KOL, but not charged
    pub over_cap: Vec<BillableEvent>,
    /// Amount actually billed this period
    pub billed: Cents,
    /// Amount waived by the cap — shown on dashboards as "free conversions this month"
    pub waived: Cents,
}

/// Apply a monthly cap to cleared events in a billing period.
///
/// Two deliberate design choices:
///
/// 1. Authoritative calculation happens at month-end settlement (here), not via Redis counters at
///    write time. Redis counts are unreliable under concurrency, restarts, and reversal rollbacks —
///    fine for soft real-time hints, not invoices. Bills must be recomputed from Postgres facts.
///
/// 2. Over-cap conversions are "free", not "rejected". Events stay recorded, attributed, and credited
///    to the KOL — they just skip the invoice. Better UX than "stop service after cap" and natural
///    upsell copy.
///
/// `events` must be sorted by `cleared_at` ascending — earlier conversions consume cap first; the
/// only order customers can understand. `None` cap means unlimited.
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

/// Validate and apply a status transition.
///
/// All status changes must go through here — never assign `status` directly. The state machine is
/// a core revenue constraint; scattered assignments hide bugs like "re-clear an invoiced event".
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

/// Whether an event has passed its hold period and may clear.
///
/// `Held` requires manual review and must not auto-clear with time — risk hold means "do not bill
/// until a human has looked"; auto-expiry would make holds meaningless.
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

        assert_eq!(res.billable.len(), 2, "only first two fit within cap");
        assert_eq!(res.billed, Cents(400));
        assert_eq!(res.over_cap.len(), 2);
        assert_eq!(res.waived, Cents(400));
    }

    /// Over-cap events are marked free, not dropped. They remain attributed and credited to the KOL,
    /// but skip the invoice.
    #[test]
    fn over_cap_events_are_marked_not_dropped() {
        let res = apply_cap(events(&[300, 300]), Some(Cents(300)));

        assert_eq!(res.over_cap.len(), 1);
        assert!(res.over_cap[0].over_cap, "over-cap event must be flagged");
        assert_eq!(res.over_cap[0].id, 2, "later event should be the free one");
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

    /// A single large event that would breach the cap is excluded whole — no partial billing.
    /// Partial billing shows customers "half a conversion", which matches nothing.
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
        assert_eq!(
            ev.status,
            BillableStatus::Billed,
            "illegal transition must not change status"
        );
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
        assert!(!ready_to_clear(&ev, at(10)), "must not clear during hold");

        ev.hold_until = at(9);
        assert!(
            ready_to_clear(&ev, at(10)),
            "should clear after hold expires"
        );
    }

    /// Risk-held events must not auto-clear with time — hold means "no billing until reviewed".
    #[test]
    fn held_events_never_auto_clear() {
        let mut ev = events(&[200]).pop().unwrap();
        ev.status = BillableStatus::Held;
        ev.hold_until = at(1);

        assert!(!ready_to_clear(&ev, at(23)));
    }
}
