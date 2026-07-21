//! Daily ledger invariant checks.
//!
//! The ledger is append-only — once an unbalanced entry is written, there is no "fix in place" —
//! only compensating entries, and all reconciliation is wrong until then.
//!
//! Types block half the problem: unbalanced `ledger::Txn` cannot be constructed. Types cannot stop
//! "bypass Txn with raw SQL" or "half the entries written then crash". This job handles the other
//! half: **re-sum database facts every day**.
//!
//! Failure means alert, not log. Ledger mismatch implies issued invoices may be wrong — the most
//! severe class of problem in this system.

use serde::Serialize;
use sqlx::{PgPool, Row};

use crate::db;
use crate::models::Cents;

/// One invariant violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Violation {
    pub tenant_id: i64,
    pub rule: &'static str,
    pub detail: String,
}

/// Input to one check: facts aggregated from the database.
#[derive(Debug, Default, Clone)]
pub struct Facts {
    /// Unbalanced transactions: (txn_id, debit total, credit total)
    pub unbalanced_txns: Vec<(String, Cents, Cents)>,
    /// Ledger-wide debit total
    pub total_debit: Cents,
    /// Ledger-wide credit total
    pub total_credit: Cents,
    /// Sum of amounts for `billed` billable events
    pub billed_events: Cents,
    /// Net `platform_revenue` from **billable events only** (credit − debit).
    ///
    /// Must filter by `ref_type`: platform subscription fees also hit `platform_revenue` but map
    /// to no `billable_event`. Without filtering, every subscribed tenant false-alarms monthly —
    /// a noisy "ledger unbalanced" alert is ignored entirely.
    pub revenue_balance: Cents,
}

/// Check invariants. Pure function: facts in, violations out, no IO.
///
/// Three invariants (design doc §3):
///
/// 1. For every `txn_id`, `sum(D) == sum(C)` — each transaction balances
/// 2. Ledger-wide `sum(D) == sum(C)` — books balance
/// 3. Sum of `billed` event amounts == net `platform_revenue` from billable events — invoice matches ledger
///
/// Invariant 3 is easy to dismiss as "redundant" when 1 and 2 hold — internal consistency ≠
/// correctness. An event marked `billed` with no entries, or entries without `billed`, passes 1 and 2
/// but is exactly how revenue diverges.
pub fn check(tenant_id: i64, f: &Facts) -> Vec<Violation> {
    let mut out = Vec::new();

    for (txn_id, debit, credit) in &f.unbalanced_txns {
        out.push(Violation {
            tenant_id,
            rule: "txn_balanced",
            detail: format!("txn {txn_id} debit {debit} / credit {credit}"),
        });
    }

    if f.total_debit != f.total_credit {
        out.push(Violation {
            tenant_id,
            rule: "ledger_balanced",
            detail: format!("ledger debit {} / credit {}", f.total_debit, f.total_credit),
        });
    }

    if f.billed_events != f.revenue_balance {
        out.push(Violation {
            tenant_id,
            rule: "revenue_matches_billed",
            detail: format!(
                "billed events total {} / platform_revenue net {}",
                f.billed_events, f.revenue_balance
            ),
        });
    }

    out
}

/// Check all tenants. Returns all violations; caller handles alerting.
pub async fn run(pool: &PgPool) -> anyhow::Result<Vec<Violation>> {
    let mut all = Vec::new();
    for tenant_id in super::all_tenant_ids(pool).await? {
        let facts = gather(pool, tenant_id).await?;
        let violations = check(tenant_id, &facts);
        if violations.is_empty() {
            tracing::info!(tenant_id, "ledger audit passed");
        } else {
            for v in &violations {
                // Error level: this log should wire straight to alerting, not sit in logs only.
                tracing::error!(tenant_id, rule = v.rule, detail = %v.detail, "ledger invariant violated");
            }
        }
        all.extend(violations);
    }
    Ok(all)
}

async fn gather(pool: &PgPool, tenant_id: i64) -> anyhow::Result<Facts> {
    let mut tx = db::begin_tenant_tx(pool, tenant_id).await?;

    let rows = sqlx::query(
        r#"
        SELECT txn_id::text AS txn_id,
               COALESCE(sum(amount_cents) FILTER (WHERE direction = 'D'), 0)::bigint AS debit,
               COALESCE(sum(amount_cents) FILTER (WHERE direction = 'C'), 0)::bigint AS credit
          FROM ledger_entry
         GROUP BY txn_id
        HAVING COALESCE(sum(amount_cents) FILTER (WHERE direction = 'D'), 0)
            <> COALESCE(sum(amount_cents) FILTER (WHERE direction = 'C'), 0)
        "#,
    )
    .fetch_all(&mut *tx)
    .await?;

    let unbalanced_txns = rows
        .into_iter()
        .map(|r| {
            Ok::<_, sqlx::Error>((
                r.try_get::<String, _>("txn_id")?,
                Cents(r.try_get::<i64, _>("debit")?),
                Cents(r.try_get::<i64, _>("credit")?),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let totals = sqlx::query(
        r#"
        -- Explicit ::bigint everywhere: Postgres sum(bigint) returns NUMERIC;
        -- omitting cast fails decode with i64 vs NUMERIC mismatch.
        SELECT COALESCE(sum(amount_cents) FILTER (WHERE direction = 'D'), 0)::bigint AS debit,
               COALESCE(sum(amount_cents) FILTER (WHERE direction = 'C'), 0)::bigint AS credit,
               -- ref_type filter: subscription fees also credit platform_revenue but match no
               -- billable_event; mixing them in triggers false invariant-3 alarms every month.
               (COALESCE(sum(amount_cents) FILTER (
                    WHERE account = 'platform_revenue' AND direction = 'C'
                      AND ref_type = 'billable_event'), 0)
              - COALESCE(sum(amount_cents) FILTER (
                    WHERE account = 'platform_revenue' AND direction = 'D'
                      AND ref_type = 'billable_event'), 0)
               )::bigint AS revenue
          FROM ledger_entry
        "#,
    )
    .fetch_one(&mut *tx)
    .await?;

    // Reversed events are `reversed` not `billed`; ledger entries net to zero via reversing postings —
    // both sides drop out naturally; no extra credit subtraction here.
    let billed = sqlx::query(
        "SELECT COALESCE(sum(amount_cents), 0)::bigint AS total FROM billable_event WHERE status = 'billed'",
    )
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Facts {
        unbalanced_txns,
        total_debit: Cents(totals.try_get("debit")?),
        total_credit: Cents(totals.try_get("credit")?),
        billed_events: Cents(billed.try_get("total")?),
        revenue_balance: Cents(totals.try_get("revenue")?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn balanced() -> Facts {
        Facts {
            unbalanced_txns: vec![],
            total_debit: Cents(1000),
            total_credit: Cents(1000),
            billed_events: Cents(1000),
            revenue_balance: Cents(1000),
        }
    }

    #[test]
    fn a_healthy_ledger_produces_no_violations() {
        assert!(check(1, &balanced()).is_empty());
    }

    #[test]
    fn flags_an_unbalanced_transaction() {
        let f = Facts {
            unbalanced_txns: vec![("abc-123".into(), Cents(200), Cents(100))],
            ..balanced()
        };
        let v = check(1, &f);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "txn_balanced");
        assert!(v[0].detail.contains("abc-123"));
    }

    #[test]
    fn flags_a_global_imbalance() {
        let f = Facts {
            total_credit: Cents(900),
            ..balanced()
        };
        assert!(check(1, &f).iter().any(|v| v.rule == "ledger_balanced"));
    }

    /// Ledger internally consistent but invoice wrong: event marked billed with no entries.
    /// Invariants 1 and 2 miss this — the usual cause of revenue mismatch.
    #[test]
    fn flags_billed_events_without_matching_revenue() {
        let f = Facts {
            billed_events: Cents(1200),
            ..balanced()
        };
        let v = check(1, &f);
        assert_eq!(v.len(), 1, "only invariant 3 should fire");
        assert_eq!(v[0].rule, "revenue_matches_billed");
    }

    #[test]
    fn reports_every_broken_invariant_not_just_the_first() {
        let f = Facts {
            unbalanced_txns: vec![("t1".into(), Cents(1), Cents(2))],
            total_debit: Cents(1),
            total_credit: Cents(2),
            billed_events: Cents(5),
            revenue_balance: Cents(6),
        };
        assert_eq!(check(1, &f).len(), 3, "should report all three issues");
    }

    /// Platform subscription fees hit `platform_revenue` but no billable event.
    /// Including them in `revenue_balance` false-alarms every subscribed tenant monthly —
    /// noisy "ledger unbalanced" alerts get ignored.
    #[test]
    fn subscription_revenue_is_not_compared_against_billed_events() {
        let f = Facts {
            // Ledger totals include 9900 subscription + 200 CPA, but revenue_balance is CPA only
            total_debit: Cents(10_100),
            total_credit: Cents(10_100),
            billed_events: Cents(200),
            revenue_balance: Cents(200),
            unbalanced_txns: vec![],
        };
        assert!(check(1, &f).is_empty(), "subscription fee must not count as invoice mismatch");
    }

    #[test]
    fn an_empty_ledger_is_valid() {
        assert!(check(1, &Facts::default()).is_empty());
    }
}
