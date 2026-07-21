//! Monetisation postback: main app reports an IAP transaction to us.
//!
//! **In MVP this path feeds analytics only, not billing.** Structural reason: billing on customer-
//! reported IAP means one missed postback is underpayment we almost cannot detect — pure take-rate
//! has that weakness in IAP. MVP CPA is built on redemption (facts we can confirm); postbacks measure
//! LTV and KOL quality.
//!
//! The switch is not `if MVP` but whether `pricing_config.cpa_rates` includes an `iap_purchase` rate.
//! When CPA is validated and GMV share opens, change pricing config, not ship code (same C4 pattern).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use super::policy::Policy;
use crate::db;
use crate::models::{event_type, BillableStatus, Cents};

/// Note there is **no** "no attribution" error variant. Most main-app users are not from our channel;
/// missing attribution is normal — accept the postback with `attributed = false`. Making it an error
/// makes clients retry as failures.
#[derive(Debug, thiserror::Error)]
pub enum PostbackError {
    #[error("attribution: invalid amount")]
    BadAmount,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// One monetisation postback.
#[derive(Debug, Clone, Deserialize)]
pub struct Purchase {
    pub app_user_id: String,
    /// Main-app-side unique transaction ID; idempotency key.
    pub transaction_id: String,
    /// Amount in smallest currency unit.
    pub amount: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub occurred_at: DateTime<Utc>,
}

fn default_currency() -> String {
    "USD".into()
}

/// Postback handling result.
#[derive(Debug, Clone, Serialize)]
pub struct PostbackResult {
    /// Whether this user is attributed to a KOL.
    pub attributed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kol_id: Option<i64>,
    /// Whether a billable event was created. Always false in MVP — see module docs.
    pub billable: bool,
    /// Idempotent hit: this transaction was already received.
    pub idempotent: bool,
}

/// Postback service.
pub struct Service {
    pool: PgPool,
    policy: Policy,
}

impl Service {
    pub fn new(pool: PgPool, policy: Policy) -> Self {
        Service { pool, policy }
    }

    /// Record one monetisation postback.
    pub async fn record(
        &self,
        tenant_id: i64,
        p: &Purchase,
        now: DateTime<Utc>,
    ) -> Result<PostbackResult, PostbackError> {
        if p.amount < 0 {
            return Err(PostbackError::BadAmount);
        }

        let mut tx = db::begin_tenant_tx(&self.pool, tenant_id).await?;

        // ---- idempotency: duplicate delivery is expected — always return first result ----
        if let Some(row) = sqlx::query(
            "SELECT attribution_id, billable_event_id FROM purchase_event WHERE transaction_id = $1",
        )
        .bind(&p.transaction_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let attribution_id: Option<i64> = row.try_get("attribution_id")?;
            let billable_event_id: Option<i64> = row.try_get("billable_event_id")?;
            let kol_id = match attribution_id {
                Some(id) => kol_of(&mut tx, id).await?,
                None => None,
            };
            tx.commit().await?;
            return Ok(PostbackResult {
                attributed: attribution_id.is_some(),
                kol_id,
                billable: billable_event_id.is_some(),
                idempotent: true,
            });
        }

        // ---- attribution lookup ----
        let attribution = sqlx::query(
            r#"
            SELECT a.id, a.kol_id, a.is_billable, a.locked_until
              FROM attribution a JOIN player pl ON pl.id = a.player_id
             WHERE pl.app_user_id = $1
            "#,
        )
        .bind(&p.app_user_id)
        .fetch_optional(&mut *tx)
        .await?;

        let (attribution_id, kol_id, is_billable) = match &attribution {
            Some(r) => (
                Some(r.try_get::<i64, _>("id")?),
                Some(r.try_get::<i64, _>("kol_id")?),
                r.try_get::<bool, _>("is_billable")?,
            ),
            None => (None, None, false),
        };

        // ---- billability ----
        // All three required: has attribution, attribution is billable (C1), event type has a rate.
        // Third condition is the GMV-share switch.
        let rate = self.cpa_rate(&mut tx, tenant_id, now).await?;
        let billable_event_id = match (attribution_id, is_billable, rate) {
            (Some(aid), true, Some(rate)) if rate.is_positive() => Some(
                self.insert_billable(&mut tx, tenant_id, aid, p, rate, now)
                    .await?,
            ),
            _ => None,
        };

        sqlx::query(
            r#"
            INSERT INTO purchase_event
              (tenant_id, attribution_id, app_user_id, transaction_id,
               amount_cents, currency, occurred_at, received_at, billable_event_id)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
            ON CONFLICT (tenant_id, transaction_id) DO NOTHING
            "#,
        )
        .bind(tenant_id)
        .bind(attribution_id)
        .bind(&p.app_user_id)
        .bind(&p.transaction_id)
        .bind(p.amount)
        .bind(&p.currency)
        .bind(p.occurred_at)
        .bind(now)
        .bind(billable_event_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(PostbackResult {
            attributed: attribution_id.is_some(),
            kol_id,
            billable: billable_event_id.is_some(),
            idempotent: false,
        })
    }

    /// `iap_purchase` unit rate. Unconfigured means GMV share is off.
    async fn cpa_rate(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: i64,
        now: DateTime<Utc>,
    ) -> Result<Option<Cents>, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT (cpa_rates->>$1)::bigint AS rate
              FROM pricing_config
             WHERE (tenant_id = $2 OR tenant_id IS NULL)
               AND effective_from <= $3
               AND (effective_to IS NULL OR effective_to > $3)
             -- tenant-specific pricing overrides global default
             ORDER BY tenant_id NULLS LAST, effective_from DESC
             LIMIT 1
            "#,
        )
        .bind(event_type::IAP_PURCHASE)
        .bind(tenant_id)
        .bind(now)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(row
            .and_then(|r| r.try_get::<Option<i64>, _>("rate").ok().flatten())
            .map(Cents))
    }

    async fn insert_billable(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: i64,
        attribution_id: i64,
        p: &Purchase,
        amount: Cents,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        // 35-day hold covers App Store refund window — otherwise we pay the KOL and only then
        // discover a refunded order.
        let hold_until = p.occurred_at + self.policy.hold_period(event_type::IAP_PURCHASE);

        let row = sqlx::query(
            r#"
            INSERT INTO billable_event
              (tenant_id, attribution_id, event_type, external_id, status,
               amount_cents, currency, occurred_at, received_at, hold_until)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
            ON CONFLICT (tenant_id, event_type, external_id)
              DO UPDATE SET received_at = billable_event.received_at
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind(attribution_id)
        .bind(event_type::IAP_PURCHASE)
        .bind(&p.transaction_id)
        .bind(BillableStatus::Pending)
        .bind(amount.0)
        .bind(&p.currency)
        .bind(p.occurred_at)
        .bind(now)
        .bind(hold_until)
        .fetch_one(&mut **tx)
        .await?;
        row.try_get("id")
    }
}

async fn kol_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    attribution_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query("SELECT kol_id FROM attribution WHERE id = $1")
        .bind(attribution_id)
        .fetch_optional(&mut **tx)
        .await?;
    row.map(|r| r.try_get("kol_id")).transpose()
}
