//! Invoice payment push: hand finalized invoices to the payments provider.
//!
//! Runs after `settle` (which leaves invoices in `draft`). Deliberately a **separate**,
//! re-runnable job: a gateway outage must never block invoice generation, and a push that half
//! succeeded must be safe to retry. Idempotency has two layers — `stripe_invoice_id IS NULL`
//! selects only unsent invoices, and [`InvoicePush::idempotency_key`] dedupes at the provider if a
//! crash lands between a successful remote push and the local status update.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::db;
use crate::payments::{InvoicePush, PaymentGateway, PushLine, invoice_status};

/// Outcome of a push run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub pushed: usize,
    pub failed: usize,
}

/// Push all unsent invoices for every tenant.
pub async fn run<G: PaymentGateway>(
    pool: &PgPool,
    gateway: &G,
    now: DateTime<Utc>,
) -> anyhow::Result<Summary> {
    let mut summary = Summary::default();
    for tenant_id in super::all_tenant_ids(pool).await? {
        match push_for_tenant(pool, gateway, tenant_id, now).await {
            Ok(n) => summary.pushed += n,
            // One tenant's gateway failure must not block others — same isolation as settle.
            Err(e) => {
                summary.failed += 1;
                tracing::error!(tenant_id, error = %e, "invoice push failed");
            }
        }
    }
    Ok(summary)
}

async fn push_for_tenant<G: PaymentGateway>(
    pool: &PgPool,
    gateway: &G,
    tenant_id: i64,
    now: DateTime<Utc>,
) -> anyhow::Result<usize> {
    let mut pushed = 0;

    // Collect candidate ids first (short read), then push each in its own transaction — so the
    // gateway call never holds a lock across more than one invoice.
    let ids = {
        let mut tx = db::begin_tenant_tx(pool, tenant_id).await?;
        let rows = sqlx::query(
            "SELECT id FROM invoice WHERE status = $1 AND stripe_invoice_id IS NULL ORDER BY id",
        )
        .bind(invoice_status::DRAFT)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(|r| r.try_get::<i64, _>("id"))
            .collect::<Result<Vec<_>, _>>()?
    };

    for id in ids {
        if push_one(pool, gateway, tenant_id, id, now).await? {
            pushed += 1;
        }
    }
    Ok(pushed)
}

/// Push a single invoice. Returns `false` if it was already sent by a concurrent run.
async fn push_one<G: PaymentGateway>(
    pool: &PgPool,
    gateway: &G,
    tenant_id: i64,
    invoice_id: i64,
    now: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let mut tx = db::begin_tenant_tx(pool, tenant_id).await?;

    // Re-read under FOR UPDATE: a concurrent push may have sent this invoice since we listed ids.
    let Some(inv) = sqlx::query(
        r#"
        SELECT currency, total_cents
          FROM invoice
         WHERE id = $1 AND status = $2 AND stripe_invoice_id IS NULL
         FOR UPDATE
        "#,
    )
    .bind(invoice_id)
    .bind(invoice_status::DRAFT)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(false);
    };

    let currency: String = inv.try_get("currency")?;
    let total_cents: i64 = inv.try_get("total_cents")?;

    let lines = sqlx::query(
        "SELECT description, quantity, unit_cents, amount_cents FROM invoice_line WHERE invoice_id = $1 ORDER BY id",
    )
    .bind(invoice_id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| {
        Ok(PushLine {
            description: r.try_get("description")?,
            quantity: r.try_get("quantity")?,
            unit_cents: r.try_get("unit_cents")?,
            amount_cents: r.try_get("amount_cents")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()?;

    let push = InvoicePush::new(invoice_id, tenant_id, currency, total_cents, lines);

    // The gateway call happens inside the transaction, so a failure rolls back to draft and the
    // next run retries cleanly. At month-end batch scale (one invoice per tenant) holding the row
    // lock across one provider call is fine.
    let receipt = gateway.push_invoice(&push).await?;

    sqlx::query(
        "UPDATE invoice SET status = $2, stripe_invoice_id = $3, pushed_at = $4 WHERE id = $1",
    )
    .bind(invoice_id)
    .bind(invoice_status::OPEN)
    .bind(&receipt.external_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    tracing::info!(
        tenant_id,
        invoice_id,
        external_id = %receipt.external_id,
        "invoice pushed"
    );
    Ok(true)
}
