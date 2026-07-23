//! Claim-code redemption: the chain's sole stitching point.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use super::claim_code::{is_valid_claim_code, normalize_claim_code};
use super::policy::Policy;
use crate::db;
use crate::models::{AttributionMethod, BillableStatus, ClaimStatus, event_type};
use crate::risk;

#[derive(Debug, thiserror::Error)]
pub enum RedeemError {
    #[error("attribution: invalid claim code format")]
    Malformed,
    #[error("attribution: claim code not found")]
    NotFound,
    #[error("attribution: claim code already redeemed")]
    AlreadyUsed,
    #[error("attribution: claim code expired")]
    Expired,
    #[error("attribution: app user already attributed to another channel")]
    AlreadyBound,
    #[error("attribution: risk check rejected ({0})")]
    RiskDenied(&'static str),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Redemption request.
#[derive(Debug, Clone)]
pub struct RedeemRequest {
    pub tenant_id: i64,
    pub code: String,
    pub app_user_id: String,
    pub device_id: Option<String>,
    pub ip: Option<String>,
    pub now: DateTime<Utc>,
}

/// Redemption result returned to the main app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedeemResult {
    pub attributed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kol_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<AttributionMethod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,
    /// Attribution holds and reward may proceed, but billing is risk-held.
    /// Main app need not treat differently — field aligns customer-side logs with our ledger.
    pub held: bool,
}

/// Attribution service.
pub struct Service {
    pool: PgPool,
    policy: Policy,
}

impl Service {
    pub fn new(pool: PgPool, policy: Policy) -> Self {
        Service { pool, policy }
    }

    /// Redeem a claim code.
    ///
    /// Sole stitching point on the chain: TG identity (`tg_user_id`) and app identity (`app_user_id`)
    /// bind here; attribution and billable event are created together.
    ///
    /// **Entire flow must run in one transaction.** If binding succeeds but attribution write fails,
    /// the user can never be attributed correctly — the code is consumed with no second chance.
    pub async fn redeem(&self, req: &RedeemRequest) -> Result<RedeemResult, RedeemError> {
        let code = normalize_claim_code(&req.code);
        if !is_valid_claim_code(&code) {
            return Err(RedeemError::Malformed);
        }

        let mut tx = db::begin_tenant_tx(&self.pool, req.tenant_id).await?;

        // FOR UPDATE on this row: concurrent submits (double-tap, client retry) block until the first
        // commit, then see redeemed. Without it, both txs read issued and each writes attribution + billing.
        let claim = sqlx::query(
            r#"
            SELECT c.id, c.player_id, c.campaign_id, c.link_id, c.kol_id,
                   c.status, c.issued_at, c.expires_at, p.tg_user_id
              FROM claim_code c
              JOIN player p ON p.id = c.player_id
             WHERE c.code = $1
             FOR UPDATE OF c
            "#,
        )
        .bind(&code)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RedeemError::NotFound)?;

        let claim_id: i64 = claim.try_get("id")?;
        let player_id: i64 = claim.try_get("player_id")?;
        let campaign_id: i64 = claim.try_get("campaign_id")?;
        let link_id: i64 = claim.try_get("link_id")?;
        let kol_id: i64 = claim.try_get("kol_id")?;
        let status: ClaimStatus = claim.try_get("status")?;
        let issued_at: DateTime<Utc> = claim.try_get("issued_at")?;
        let expires_at: DateTime<Utc> = claim.try_get("expires_at")?;
        let tg_user_id: i64 = claim.try_get("tg_user_id")?;

        match status {
            ClaimStatus::Redeemed => return Err(RedeemError::AlreadyUsed),
            ClaimStatus::Issued => {}
            // expired / voided are equivalent to not found for callers — do not leak internal state
            _ => return Err(RedeemError::NotFound),
        }
        if req.now > expires_at {
            return Err(RedeemError::Expired);
        }

        // ---- risk L1 ----
        let device_player_count = match &req.device_id {
            Some(d) if !d.is_empty() => {
                sqlx::query("SELECT count(*) AS n FROM player WHERE $1 = ANY(device_ids)")
                    .bind(d)
                    .fetch_one(&mut *tx)
                    .await?
                    .try_get::<i64, _>("n")?
            }
            _ => 0,
        };
        let ip_redeem_today = match &req.ip {
            Some(ip) if !ip.is_empty() => sqlx::query(
                r#"
                SELECT count(*) AS n FROM risk_signal
                 WHERE stage = 'redeem' AND ip = $1::inet
                   AND created_at >= date_trunc('day', $2::timestamptz)
                "#,
            )
            .bind(ip)
            .bind(req.now)
            .fetch_one(&mut *tx)
            .await?
            .try_get::<i64, _>("n")?,
            _ => 0,
        };

        let verdict = risk::check_redeem(&risk::RedeemInput {
            device_player_count,
            ip_redeem_today,
            tg_user_id,
            click_to_redeem: Some(req.now - issued_at),
        });
        if verdict.action == risk::Action::Deny {
            return Err(RedeemError::RiskDenied(verdict.rule));
        }

        // ---- status advance and identity binding ----
        sqlx::query("UPDATE claim_code SET status = 'redeemed', redeemed_at = $2 WHERE id = $1")
            .bind(claim_id)
            .bind(req.now)
            .execute(&mut *tx)
            .await?;

        let device = req.device_id.clone().unwrap_or_default();
        // app_user_id is unique. If another Player already bound this app user, same account is
        // attempting a second attribution — reject on conflict.
        let bound = sqlx::query(
            r#"
            UPDATE player
               SET app_user_id = $2,
                   device_ids  = CASE WHEN $3 = '' OR $3 = ANY(device_ids)
                                      THEN device_ids ELSE array_append(device_ids, $3) END
             WHERE id = $1
               AND (app_user_id IS NULL OR app_user_id = $2)
            "#,
        )
        .bind(player_id)
        .bind(&req.app_user_id)
        .bind(&device)
        .execute(&mut *tx)
        .await?;
        if bound.rows_affected() == 0 {
            return Err(RedeemError::AlreadyBound);
        }

        // ---- attribution ----
        let method = AttributionMethod::DeterministicCode;
        let evidence = serde_json::json!({
            "claim_code":  code,
            "claim_id":    claim_id,
            "app_user_id": req.app_user_id,
            "device_id":   req.device_id,
            "ip":          req.ip,
            "issued_at":   issued_at,
            "redeemed_at": req.now,
            "risk":        verdict,
        });

        let inserted = sqlx::query(
            r#"
            INSERT INTO attribution
              (tenant_id, player_id, kol_id, campaign_id, link_id, method, confidence,
               is_billable, policy_version, touch_at, attributed_at, locked_until, evidence)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
            ON CONFLICT (tenant_id, player_id) DO NOTHING
            RETURNING id
            "#,
        )
        .bind(req.tenant_id)
        .bind(player_id)
        .bind(kol_id)
        .bind(campaign_id)
        .bind(link_id)
        .bind(method)
        .bind(method.confidence())
        .bind(method.is_billable())
        .bind(self.policy.version)
        .bind(issued_at)
        .bind(req.now)
        .bind(req.now + self.policy.lock_period)
        .bind(&evidence)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(row) = inserted else {
            // Single-attribution constraint: Player already belongs to a KOL. During lock period keep
            // existing attribution; do not create a new billable event.
            let existing =
                sqlx::query("SELECT id, kol_id, campaign_id FROM attribution WHERE player_id = $1")
                    .bind(player_id)
                    .fetch_one(&mut *tx)
                    .await?;
            tx.commit().await?;
            return Ok(RedeemResult {
                attributed: true,
                kol_id: Some(existing.try_get("kol_id")?),
                campaign_id: Some(existing.try_get("campaign_id")?),
                method: Some(method),
                policy_version: Some(self.policy.version.to_string()),
                held: false,
            });
        };
        let attribution_id: i64 = row.try_get("id")?;

        // ---- billable event ----
        // Only billable attributions create billing events (constraint C1).
        let mut held = false;
        if method.is_billable() {
            let (status, reason) = if verdict.action == risk::Action::Hold {
                (BillableStatus::Held, Some(verdict.rule))
            } else {
                (BillableStatus::Pending, None)
            };
            held = status == BillableStatus::Held;

            // external_id is claim_id: redemption is a fact we confirm — MVP CPA has no customer under-report leak.
            sqlx::query(
                r#"
                INSERT INTO billable_event
                  (tenant_id, attribution_id, event_type, external_id, status,
                   amount_cents, currency, occurred_at, received_at, hold_until, status_reason)
                VALUES ($1,$2,$3,$4,$5,
                        COALESCE((SELECT (cpa_rates->>$3)::bigint FROM pricing_config
                                   WHERE (tenant_id = $1 OR tenant_id IS NULL)
                                     AND effective_from <= $6
                                     AND (effective_to IS NULL OR effective_to > $6)
                                   -- tenant-specific pricing overrides global default
                                   ORDER BY tenant_id NULLS LAST, effective_from DESC
                                   LIMIT 1), 0),
                        'USD', $6, $6, $7, $8)
                ON CONFLICT (tenant_id, event_type, external_id) DO NOTHING
                "#,
            )
            .bind(req.tenant_id)
            .bind(attribution_id)
            .bind(event_type::ACTIVATION)
            .bind(format!("claim:{claim_id}"))
            .bind(status)
            .bind(req.now)
            .bind(req.now + self.policy.hold_period(event_type::ACTIVATION))
            .bind(reason)
            .execute(&mut *tx)
            .await?;
        }

        // ---- L2 signal capture: store only, decide later — cannot backfill ----
        sqlx::query(
            r#"
            INSERT INTO risk_signal (tenant_id, player_id, stage, ip, device_id, latency_ms, attrs)
            VALUES ($1,$2,'redeem', NULLIF($3,'')::inet, $4, $5, $6)
            "#,
        )
        .bind(req.tenant_id)
        .bind(player_id)
        .bind(req.ip.clone().unwrap_or_default())
        .bind(&device)
        .bind((req.now - issued_at).num_milliseconds())
        .bind(serde_json::json!({ "rule": verdict.rule, "action": verdict.action }))
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(RedeemResult {
            attributed: true,
            kol_id: Some(kol_id),
            campaign_id: Some(campaign_id),
            method: Some(method),
            policy_version: Some(self.policy.version.to_string()),
            held,
        })
    }
}
