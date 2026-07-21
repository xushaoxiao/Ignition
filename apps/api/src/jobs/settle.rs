//! Month-end settlement: issue invoices.
//!
//! **Usage is not billed in real time.** CPA events have a 7-day hold; real-time billing would
//! require constant reversals and terrible customer experience. Month-end batch settlement aligns
//! naturally with holds (design doc §5.1).
//!
//! First place ledger, caps, and the state machine enter the online path together — before this,
//! `ledger` and `billing::apply_cap` were test-only.

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool, Row};

use crate::models::{BillableEvent, BillableStatus, Cents};
use crate::{billing, db, ledger};

/// Billing period. Half-open interval [start, end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl Period {
    /// Previous calendar month relative to `now` — T+1 month-end jobs settle the prior month.
    pub fn previous_month(now: DateTime<Utc>) -> Period {
        let d = now.date_naive();
        let this_month = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).expect("day 1 is always a valid date");
        Period {
            start: prev_month_first_day(this_month),
            end: this_month,
        }
    }
}

fn prev_month_first_day(first_of_month: NaiveDate) -> NaiveDate {
    let (y, m) = (first_of_month.year(), first_of_month.month());
    if m == 1 {
        NaiveDate::from_ymd_opt(y - 1, 12, 1).expect("1 December is a valid date")
    } else {
        NaiveDate::from_ymd_opt(y, m - 1, 1).expect("1st of previous month is a valid date")
    }
}

/// One invoice line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Line {
    pub kind: &'static str,
    pub description: String,
    pub quantity: i64,
    pub unit_cents: Cents,
    pub amount_cents: Cents,
}

pub mod line_kind {
    pub const PLATFORM_FEE: &str = "platform_fee";
    pub const CPA: &str = "cpa";
    pub const CREDIT: &str = "credit";
}

/// Settlement inputs: facts read from the database.
#[derive(Debug, Default)]
pub struct Inputs {
    /// Cleared events awaiting settlement, sorted by `cleared_at` ascending — earlier conversions
    /// consume cap first; the only order customers can understand.
    pub cleared: Vec<BillableEvent>,
    /// Monthly cap. `None` means unlimited.
    pub cap: Option<Cents>,
    /// Monthly platform fee.
    pub platform_fee: Cents,
    /// Amount reversed on previously invoiced events, credited this period.
    pub credit: Cents,
}

/// Settlement output.
#[derive(Debug, Default)]
pub struct Bill {
    pub lines: Vec<Line>,
    pub subtotal: Cents,
    pub credit: Cents,
    pub total: Cents,
    /// Events on the invoice — transition to `billed` and post ledger entries.
    pub billable: Vec<BillableEvent>,
    /// Over-cap events: still attributed and credited to the KOL, but not charged.
    pub over_cap: Vec<BillableEvent>,
    /// Amount waived by the cap — shown on dashboards as "free conversions this month".
    pub waived: Cents,
}

/// Assemble an invoice. Pure function, no IO — invoice arithmetic must be line-by-line reproducible.
///
/// On **negative totals**: when credits exceed usage, `total` goes negative. That is a credit note,
/// not an error — Stripe records customer credit and auto-applies next period. Deliberately not
/// clamped to zero: clamping silently keeps overpayment.
pub fn assemble(inputs: Inputs) -> Bill {
    let mut bill = Bill::default();

    if inputs.platform_fee.is_positive() {
        bill.lines.push(Line {
            kind: line_kind::PLATFORM_FEE,
            description: "Platform subscription fee".into(),
            quantity: 1,
            unit_cents: inputs.platform_fee,
            amount_cents: inputs.platform_fee,
        });
        bill.subtotal += inputs.platform_fee;
    }

    let capped = billing::apply_cap(inputs.cleared, inputs.cap);
    if !capped.billable.is_empty() {
        // Unit price from event amount — consistent within a period; first row is enough. Amounts are
        // frozen on events; do not re-query pricing — pricing changes must not rewrite history.
        let unit = capped.billable[0].amount_cents;
        bill.lines.push(Line {
            kind: line_kind::CPA,
            description: format!("Performance share · {} deterministic conversions", capped.billable.len()),
            quantity: capped.billable.len() as i64,
            unit_cents: unit,
            amount_cents: capped.billed,
        });
        bill.subtotal += capped.billed;
    }

    if inputs.credit.is_positive() {
        bill.lines.push(Line {
            kind: line_kind::CREDIT,
            description: "Prior period reversal".into(),
            quantity: 1,
            unit_cents: inputs.credit,
            amount_cents: inputs.credit,
        });
    }

    bill.credit = inputs.credit;
    bill.total = Cents(bill.subtotal.0 - inputs.credit.0);
    bill.billable = capped.billable;
    bill.over_cap = capped.over_cap;
    bill.waived = capped.waived;
    bill
}

/// Settle one billing period for all tenants.
pub async fn run(pool: &PgPool, period: Period, now: DateTime<Utc>) -> anyhow::Result<()> {
    for tenant_id in super::all_tenant_ids(pool).await? {
        match run_for_tenant(pool, tenant_id, period, now).await {
            Ok(Some(id)) => tracing::info!(tenant_id, invoice_id = id, "invoice generated"),
            Ok(None) => tracing::info!(tenant_id, "invoice already exists, skipping"),
            // One tenant failing must not block others — month-end window is tight; stalling 19 good
            // invoices because tenant 20 has bad data is the worst outcome.
            Err(e) => tracing::error!(tenant_id, error = %e, "settlement failed"),
        }
    }
    Ok(())
}

async fn run_for_tenant(
    pool: &PgPool,
    tenant_id: i64,
    period: Period,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<i64>> {
    let mut tx = db::begin_tenant_tx(pool, tenant_id).await?;

    // Idempotent: one invoice per period. Rerun skips instead of issuing a second bill.
    let exists = sqlx::query("SELECT id FROM invoice WHERE period_start = $1")
        .bind(period.start)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_some() {
        return Ok(None);
    }

    let pricing = load_pricing(&mut tx, tenant_id, now).await?;

    // Select events where `invoice_id IS NULL`, not "cleared_at in this period".
    // Late-cleared events matter: conversion last month, manual review finished this month —
    // period-window selection drops them forever; uninvoiced selection picks them up next run.
    let rows = sqlx::query(
        r#"
        SELECT id, tenant_id, attribution_id, event_type, external_id, status,
               amount_cents, currency, over_cap, occurred_at, received_at,
               hold_until, cleared_at, billed_at, invoice_id, status_reason
          FROM billable_event
         WHERE status = 'cleared' AND invoice_id IS NULL AND cleared_at < $1
         ORDER BY cleared_at
         FOR UPDATE
        "#,
    )
    .bind(period_end_ts(period))
    .fetch_all(&mut *tx)
    .await?;

    let cleared = rows
        .iter()
        .map(BillableEvent::from_row)
        .collect::<Result<Vec<_>, _>>()?;

    let credit = load_pending_credit(&mut tx, period).await?;

    let bill = assemble(Inputs {
        cleared,
        cap: pricing.monthly_cap,
        platform_fee: pricing.platform_fee,
        credit,
    });

    if bill.lines.is_empty() && bill.over_cap.is_empty() {
        return Ok(None);
    }

    let invoice_id: i64 = sqlx::query(
        r#"
        INSERT INTO invoice
          (tenant_id, period_start, period_end, subtotal_cents, credit_cents,
           total_cents, currency, status, created_at)
        VALUES ($1,$2,$3,$4,$5,$6,$7,'draft',$8)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(period.start)
    .bind(period.end)
    .bind(bill.subtotal.0)
    .bind(bill.credit.0)
    .bind(bill.total.0)
    .bind(&pricing.currency)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?
    .try_get("id")?;

    for line in &bill.lines {
        sqlx::query(
            r#"
            INSERT INTO invoice_line
              (tenant_id, invoice_id, kind, description, quantity, unit_cents, amount_cents)
            VALUES ($1,$2,$3,$4,$5,$6,$7)
            "#,
        )
        .bind(tenant_id)
        .bind(invoice_id)
        .bind(line.kind)
        .bind(&line.description)
        .bind(line.quantity)
        .bind(line.unit_cents.0)
        .bind(line.amount_cents.0)
        .execute(&mut *tx)
        .await?;
    }

    // ---- billable events → billed + double-entry postings ----
    for ev in &bill.billable {
        let mut ev = ev.clone();
        billing::transition(&mut ev, BillableStatus::Billed, None, now)
            .map_err(|e| anyhow::anyhow!(e))?;

        sqlx::query(
            "UPDATE billable_event SET status = $2, billed_at = $3, invoice_id = $4, over_cap = false WHERE id = $1",
        )
        .bind(ev.id)
        .bind(ev.status)
        .bind(ev.billed_at)
        .bind(invoice_id)
        .execute(&mut *tx)
        .await?;

        let txn = ledger::charge_cpa(tenant_id, &ev)?;
        write_txn(&mut tx, &txn, now).await?;
    }

    // ---- over cap: attach to invoice but do not charge ----
    // Status stays cleared — it was cleared, this line is just free. invoice_id prevents re-pick
    // next period and lets customers see "how much was free" in detail.
    for ev in &bill.over_cap {
        sqlx::query("UPDATE billable_event SET over_cap = true, invoice_id = $2 WHERE id = $1")
            .bind(ev.id)
            .bind(invoice_id)
            .execute(&mut *tx)
            .await?;
    }

    // ---- platform fee posting ----
    if bill.subtotal.is_positive() && pricing.platform_fee.is_positive() {
        if let Some(sub_id) = pricing.subscription_id {
            let txn = ledger::charge_platform_fee(
                tenant_id,
                sub_id,
                pricing.platform_fee,
                &pricing.currency,
            )?;
            write_txn(&mut tx, &txn, now).await?;
        }
    }

    // ---- mark reversal credit applied so next period does not double-deduct ----
    sqlx::query(
        "UPDATE billable_event SET credited_invoice_id = $1 WHERE status = 'reversed' AND credited_invoice_id IS NULL AND invoice_id IS NOT NULL",
    )
    .bind(invoice_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Some(invoice_id))
}

async fn write_txn(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    txn: &ledger::Txn,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    for e in txn.entries() {
        sqlx::query(
            r#"
            INSERT INTO ledger_entry
              (tenant_id, txn_id, account, direction, amount_cents, currency,
               ref_type, ref_id, created_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
            "#,
        )
        .bind(txn.tenant_id())
        .bind(txn.id())
        .bind(e.account.as_str())
        .bind(e.direction.as_str())
        .bind(e.amount.0)
        .bind(&e.currency)
        .bind(txn.ref_type())
        .bind(txn.ref_id())
        .bind(now)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

struct Pricing {
    platform_fee: Cents,
    monthly_cap: Option<Cents>,
    currency: String,
    subscription_id: Option<i64>,
}

async fn load_pricing(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: i64,
    now: DateTime<Utc>,
) -> Result<Pricing, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT platform_fee_cents, monthly_cap_cents, currency
          FROM pricing_config
         WHERE (tenant_id = $1 OR tenant_id IS NULL)
           AND effective_from <= $2
           AND (effective_to IS NULL OR effective_to > $2)
         ORDER BY tenant_id NULLS LAST, effective_from DESC
         LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await?;

    let sub = sqlx::query("SELECT id FROM subscription WHERE status <> 'canceled' LIMIT 1")
        .fetch_optional(&mut **tx)
        .await?;

    Ok(match row {
        Some(r) => Pricing {
            platform_fee: Cents(r.try_get("platform_fee_cents")?),
            monthly_cap: r.try_get::<Option<i64>, _>("monthly_cap_cents")?.map(Cents),
            currency: r.try_get("currency")?,
            subscription_id: sub.map(|s| s.try_get("id")).transpose()?,
        },
        // Missing pricing must not silently bill zero — that produces wrong invoices. Return zero
        // platform fee with cap None; upper layer skips empty invoice — "no invoice" not "$0 invoice".
        None => Pricing {
            platform_fee: Cents::ZERO,
            monthly_cap: None,
            currency: "USD".into(),
            subscription_id: None,
        },
    })
}

/// Reversal credit due this period: previously invoiced, later reversed, not yet credited on any invoice.
async fn load_pending_credit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    _period: Period,
) -> Result<Cents, sqlx::Error> {
    let row = sqlx::query(
        r#"
        -- ::bigint required: Postgres sum(bigint) returns NUMERIC
        SELECT COALESCE(sum(amount_cents), 0)::bigint AS total
          FROM billable_event
         WHERE status = 'reversed'
           AND invoice_id IS NOT NULL
           AND credited_invoice_id IS NULL
        "#,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(Cents(row.try_get("total")?))
}

fn period_end_ts(period: Period) -> DateTime<Utc> {
    period
        .end
        .and_hms_opt(0, 0, 0)
        .expect("midnight is always a valid time")
        .and_utc()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::event_type;
    use chrono::TimeZone;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, day, 0, 0, 0).unwrap()
    }

    fn cleared_events(amounts: &[i64]) -> Vec<BillableEvent> {
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
                occurred_at: at(1),
                received_at: at(1),
                hold_until: at(8),
                cleared_at: Some(at(8)),
                billed_at: None,
                invoice_id: None,
                status_reason: None,
            })
            .collect()
    }

    fn inputs(amounts: &[i64]) -> Inputs {
        Inputs {
            cleared: cleared_events(amounts),
            cap: None,
            platform_fee: Cents(9900),
            credit: Cents::ZERO,
        }
    }

    #[test]
    fn bills_platform_fee_and_usage() {
        let bill = assemble(inputs(&[200, 200, 200]));

        assert_eq!(bill.subtotal, Cents(9900 + 600));
        assert_eq!(bill.total, Cents(10500));
        assert_eq!(bill.lines.len(), 2);
        assert_eq!(bill.lines[1].quantity, 3);
        assert_eq!(bill.lines[1].unit_cents, Cents(200));
        assert_eq!(bill.billable.len(), 3);
    }

    /// Over-cap conversions remain recorded but unbilled — they land in over_cap, not subtotal.
    /// "Free after cap" not "stop service after cap" is deliberate product choice.
    #[test]
    fn over_cap_conversions_are_free_not_rejected() {
        let bill = assemble(Inputs {
            cap: Some(Cents(400)),
            ..inputs(&[200, 200, 200, 200])
        });

        assert_eq!(bill.billable.len(), 2, "only first two within cap");
        assert_eq!(bill.over_cap.len(), 2, "remaining two still recorded");
        assert_eq!(bill.waived, Cents(400), "should show waived amount");
        assert_eq!(bill.subtotal, Cents(9900 + 400));
    }

    /// Credits exceeding usage yield negative total — a credit note; must not clamp to zero or
    /// customer overpayment is swallowed.
    #[test]
    fn credit_can_exceed_usage_and_produce_a_negative_total() {
        let bill = assemble(Inputs {
            platform_fee: Cents::ZERO,
            credit: Cents(1000),
            ..inputs(&[200])
        });

        assert_eq!(bill.subtotal, Cents(200));
        assert_eq!(bill.total, Cents(-800));
        assert!(bill.lines.iter().any(|l| l.kind == line_kind::CREDIT));
    }

    #[test]
    fn no_usage_still_bills_the_platform_fee() {
        let bill = assemble(inputs(&[]));

        assert_eq!(bill.lines.len(), 1);
        assert_eq!(bill.lines[0].kind, line_kind::PLATFORM_FEE);
        assert_eq!(bill.total, Cents(9900));
    }

    #[test]
    fn nothing_to_bill_produces_no_lines() {
        let bill = assemble(Inputs {
            platform_fee: Cents::ZERO,
            ..inputs(&[])
        });
        assert!(bill.lines.is_empty());
        assert_eq!(bill.total, Cents::ZERO);
    }

    // ------------------------------------------------------------ billing period

    #[test]
    fn previous_month_is_a_half_open_range() {
        let p = Period::previous_month(Utc.with_ymd_and_hms(2026, 7, 1, 3, 0, 0).unwrap());
        assert_eq!(p.start, NaiveDate::from_ymd_opt(2026, 6, 1).unwrap());
        assert_eq!(p.end, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap());
    }

    /// Year boundary is the easiest branch to get wrong: January settles December.
    #[test]
    fn previous_month_crosses_the_year_boundary() {
        let p = Period::previous_month(Utc.with_ymd_and_hms(2027, 1, 1, 3, 0, 0).unwrap());
        assert_eq!(p.start, NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        assert_eq!(p.end, NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
    }

    #[test]
    fn previous_month_ignores_the_day_within_the_month() {
        let a = Period::previous_month(Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap());
        let b = Period::previous_month(Utc.with_ymd_and_hms(2026, 3, 28, 23, 0, 0).unwrap());
        assert_eq!(a, b);
        assert_eq!(a.start, NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
    }
}
