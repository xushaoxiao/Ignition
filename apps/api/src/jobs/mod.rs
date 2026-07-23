//! Scheduled jobs.
//!
//! Three jobs, all triggered via `ignition <config> job <name>` and run by an external
//! scheduler (cron / k8s CronJob). Deliberately not embedded in the HTTP process: billing
//! jobs need manual reruns, period selection, and isolated logs — all harder inside the
//! server process.
//!
//! | Job | Cadence | Purpose |
//! |---|---|---|
//! | `clear-holds` | Hourly | Move hold-expired events to `cleared` |
//! | `ledger-audit` | Daily | Check ledger invariants; alert on failure |
//! | `settle` | Monthly T+1 | Issue invoices: caps, credits, double-entry postings |

pub mod audit;
pub mod clear;
pub mod push;
pub mod settle;

use sqlx::{PgPool, Row};

/// List all tenant IDs.
///
/// Batch jobs span tenants, while the `tenant` table itself has no RLS (nothing to compare
/// `tenant_id` against). After collecting IDs, all reads and writes still go through
/// `db::begin_tenant_tx` — one transaction per tenant — so one tenant's bad data does not
/// block everyone else's invoice.
pub async fn all_tenant_ids(pool: &PgPool) -> Result<Vec<i64>, sqlx::Error> {
    let rows = sqlx::query("SELECT id FROM tenant ORDER BY id")
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(|r| r.try_get("id")).collect()
}
