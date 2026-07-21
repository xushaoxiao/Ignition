//! Claim-code issuance.
//!
//! Step 4 of the attribution chain: user wins in the TMA, we issue a code, they redeem it in the
//! main app. **Codes are the only billable attribution carrier on iOS** — no reliable user-level
//! deferred deep link on iOS; fingerprint accuracy collapsed after iOS 17+, so manual code entry in
//! the main app is the remaining deterministic path.
//!
//! The response includes not just the code but per-platform handoff guidance (see [`Handoff`]).

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};

use super::claim_code::new_claim_code;
use super::policy::Policy;
use crate::db;

/// Retries when generating a unique code.
///
/// 31 chars × 8 digits ≈ 8.5e11 space — collision probability is tiny, but the unique index still
/// rejects occasionally. Retry with a new code; three failures mean something other than collision.
const CODE_RETRIES: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum IssueError {
    #[error("attribution: play not found")]
    PlayNotFound,
    #[error("attribution: failed to generate claim code")]
    CodeCollision,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Issue request.
#[derive(Debug, Clone)]
pub struct IssueRequest {
    pub tenant_id: i64,
    pub player_id: i64,
    pub campaign_id: i64,
    pub link_id: i64,
    pub kol_id: i64,
    /// Which play this code is for. One code per play only.
    pub game_play_id: i64,
    pub now: DateTime<Utc>,
}

/// Per-platform handoff guidance.
///
/// Android uses Install Referrer: code embedded in the store URL, read on first launch with zero user
/// action. iOS lacks this — the code must be shown for the user to remember or copy.
#[derive(Debug, Clone, Serialize)]
pub struct Handoff {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ios_url: Option<String>,
    /// iOS must show the code itself — do not hide it because a link exists.
    pub show_code_to_user: bool,
}

/// Issue result.
#[derive(Debug, Clone, Serialize)]
pub struct IssueResult {
    pub claim_code: String,
    pub expires_at: DateTime<Utc>,
    pub handoff: Handoff,
    /// Idempotent hit: returns the first issued code.
    pub idempotent: bool,
}

/// Issuance service.
pub struct Service {
    pool: PgPool,
    policy: Policy,
}

impl Service {
    pub fn new(pool: PgPool, policy: Policy) -> Self {
        Service { pool, policy }
    }

    /// Issue a claim code for one play.
    ///
    /// Idempotency key is `game_play_id`: duplicate requests for the same play return the same code.
    /// **Must not issue a new code each time** — each code is a pending attribution carrier; repeat
    /// issuance means one play could yield multiple billable redemptions.
    pub async fn issue(&self, req: &IssueRequest) -> Result<IssueResult, IssueError> {
        let mut tx = db::begin_tenant_tx(&self.pool, req.tenant_id).await?;

        let handoff = self.handoff_for(&mut tx, req.campaign_id).await?;

        // ---- idempotency ----
        if let Some(row) =
            sqlx::query("SELECT code, expires_at FROM claim_code WHERE game_play_id = $1")
                .bind(req.game_play_id)
                .fetch_optional(&mut *tx)
                .await?
        {
            tx.commit().await?;
            return Ok(IssueResult {
                claim_code: row.try_get("code")?,
                expires_at: row.try_get("expires_at")?,
                handoff: handoff.with_code(row.try_get::<String, _>("code")?.as_str()),
                idempotent: true,
            });
        }

        // Play must belong to this player and campaign — otherwise frontend could use another play_id.
        let play_exists = sqlx::query(
            "SELECT 1 AS ok FROM game_play WHERE id = $1 AND player_id = $2 AND campaign_id = $3",
        )
        .bind(req.game_play_id)
        .bind(req.player_id)
        .bind(req.campaign_id)
        .fetch_optional(&mut *tx)
        .await?;
        if play_exists.is_none() {
            return Err(IssueError::PlayNotFound);
        }

        let expires_at = req.now + self.policy.claim_code_ttl;

        for _ in 0..CODE_RETRIES {
            let code = new_claim_code();
            let inserted = sqlx::query(
                r#"
                INSERT INTO claim_code
                  (tenant_id, code, player_id, campaign_id, link_id, kol_id,
                   game_play_id, status, issued_at, expires_at)
                VALUES ($1,$2,$3,$4,$5,$6,$7,'issued',$8,$9)
                ON CONFLICT (tenant_id, code) DO NOTHING
                RETURNING code
                "#,
            )
            .bind(req.tenant_id)
            .bind(&code)
            .bind(req.player_id)
            .bind(req.campaign_id)
            .bind(req.link_id)
            .bind(req.kol_id)
            .bind(req.game_play_id)
            .bind(req.now)
            .bind(expires_at)
            .fetch_optional(&mut *tx)
            .await?;

            if inserted.is_some() {
                tx.commit().await?;
                return Ok(IssueResult {
                    handoff: handoff.with_code(&code),
                    claim_code: code,
                    expires_at,
                    idempotent: false,
                });
            }
        }
        Err(IssueError::CodeCollision)
    }

    async fn handoff_for(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        campaign_id: i64,
    ) -> Result<HandoffTemplate, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT a.store_url_ios, a.store_url_android
              FROM campaign c JOIN app a ON a.id = c.app_id
             WHERE c.id = $1
            "#,
        )
        .bind(campaign_id)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(match row {
            Some(r) => HandoffTemplate {
                ios: r.try_get("store_url_ios")?,
                android: r.try_get("store_url_android")?,
            },
            None => HandoffTemplate::default(),
        })
    }
}

#[derive(Debug, Default, Clone)]
struct HandoffTemplate {
    ios: Option<String>,
    android: Option<String>,
}

impl HandoffTemplate {
    fn with_code(&self, code: &str) -> Handoff {
        Handoff {
            android_url: self.android.as_deref().map(|u| append_referrer(u, code)),
            ios_url: self.ios.clone(),
            // Always true. Showing the code on Android does no harm; hiding it on iOS is direct revenue loss.
            show_code_to_user: true,
        }
    }
}

/// Append claim code to the Play store URL `referrer` parameter.
///
/// Play Install Referrer delivers this value verbatim on first launch — zero user action for
/// attribution. That is why Android converts better than iOS, and why iOS manual entry UX matters.
pub fn append_referrer(store_url: &str, code: &str) -> String {
    let sep = if store_url.contains('?') { '&' } else { '?' };
    format!(
        "{store_url}{sep}referrer={}",
        urlencoding::encode(&format!("ignition_code={code}"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referrer_is_appended_with_the_right_separator() {
        assert_eq!(
            append_referrer("https://play.google.com/store/apps/details", "AB3XY9ZK"),
            "https://play.google.com/store/apps/details?referrer=ignition_code%3DAB3XY9ZK"
        );
        assert_eq!(
            append_referrer(
                "https://play.google.com/store/apps/details?id=com.demo.app",
                "AB3XY9ZK"
            ),
            "https://play.google.com/store/apps/details?id=com.demo.app&referrer=ignition_code%3DAB3XY9ZK"
        );
    }

    /// Referrer value must be URL-encoded: unescaped `=` splits query params and the app reads a truncated referrer.
    #[test]
    fn referrer_value_is_url_encoded() {
        let url = append_referrer("https://example.com/app", "AB3XY9ZK");
        assert!(url.contains("%3D"), "= not encoded: {url}");
        assert!(!url.contains("referrer=ignition_code=AB3XY9ZK"));
    }
}
