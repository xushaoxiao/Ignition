//! Auto-clear events when the hold period expires.
//!
//! `pending` events move to `cleared` after `hold_until`, becoming conversions eligible for invoicing.
//!
//! **`held` events are never cleared by this job.** Risk hold means "do not bill until a human has
//! reviewed"; auto-clearing with time turns hold into delayed clearance — better not to hold at all.
//! Enforced by `billing::ready_to_clear`, not duplicated in SQL: the state machine should have one
//! implementation.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

use crate::models::{BillableEvent, BillableStatus};
use crate::{billing, db};

/// Result of one run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub scanned: usize,
    pub cleared: usize,
}

/// Clear hold-expired events for all tenants.
pub async fn run(pool: &PgPool, now: DateTime<Utc>) -> anyhow::Result<Report> {
    let mut report = Report::default();
    for tenant_id in super::all_tenant_ids(pool).await? {
        let r = run_for_tenant(pool, tenant_id, now).await?;
        tracing::info!(
            tenant_id,
            scanned = r.scanned,
            cleared = r.cleared,
            "hold clearance released"
        );
        report.scanned += r.scanned;
        report.cleared += r.cleared;
    }
    Ok(report)
}

async fn run_for_tenant(
    pool: &PgPool,
    tenant_id: i64,
    now: DateTime<Utc>,
) -> anyhow::Result<Report> {
    let mut tx = db::begin_tenant_tx(pool, tenant_id).await?;

    // FOR UPDATE SKIP LOCKED: when the job is rerun (scheduler retry, manual rerun), two instances
    // do not fight the same rows or block each other until timeout.
    let rows = sqlx::query(
        r#"
        SELECT id, tenant_id, attribution_id, event_type, external_id, status,
               amount_cents, currency, over_cap, occurred_at, received_at,
               hold_until, cleared_at, billed_at, invoice_id, status_reason
          FROM billable_event
         WHERE status = 'pending' AND hold_until <= $1
         ORDER BY hold_until
         FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(now)
    .fetch_all(&mut *tx)
    .await?;

    let mut report = Report {
        scanned: rows.len(),
        cleared: 0,
    };

    for row in rows {
        let mut ev = BillableEvent::from_row(&row)?;

        // State machine is authoritative. SQL WHERE only narrows candidates; real decision here —
        // if the two disagree, code wins and blocks here.
        if !billing::ready_to_clear(&ev, now) {
            continue;
        }
        billing::transition(&mut ev, BillableStatus::Cleared, None, now)
            .map_err(|e| anyhow::anyhow!(e))?;

        sqlx::query(
            "UPDATE billable_event SET status = $2, cleared_at = $3 WHERE id = $1 AND status = 'pending'",
        )
        .bind(ev.id)
        .bind(ev.status)
        .bind(ev.cleared_at)
        .execute(&mut *tx)
        .await?;
        report.cleared += 1;
    }

    tx.commit().await?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Cents, event_type};
    use chrono::{TimeDelta, TimeZone};

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, day, 0, 0, 0).unwrap()
    }

    fn event(status: BillableStatus, hold_until: DateTime<Utc>) -> BillableEvent {
        BillableEvent {
            id: 1,
            tenant_id: 1,
            attribution_id: 1,
            event_type: event_type::ACTIVATION.into(),
            external_id: "claim:1".into(),
            status,
            amount_cents: Cents(200),
            currency: "USD".into(),
            over_cap: false,
            occurred_at: at(1),
            received_at: at(1),
            hold_until,
            cleared_at: None,
            billed_at: None,
            invoice_id: None,
            status_reason: None,
        }
    }

    /// Guards against held events auto-clearing with time. SQL already filters `status = 'pending'`;
    /// this is a second line — that filter is easy to loosen when editing queries.
    #[test]
    fn held_events_are_never_auto_cleared() {
        let ev = event(BillableStatus::Held, at(1));
        assert!(
            !billing::ready_to_clear(&ev, at(30)),
            "held event was auto-cleared"
        );
    }

    #[test]
    fn pending_clears_only_after_hold_expires() {
        let ev = event(BillableStatus::Pending, at(10));
        assert!(!billing::ready_to_clear(
            &ev,
            at(10) - TimeDelta::seconds(1)
        ));
        assert!(billing::ready_to_clear(&ev, at(10)));
        assert!(billing::ready_to_clear(&ev, at(11)));
    }
}
