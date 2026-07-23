//! Attribution lookup for the main app.
//!
//! Backs `GET /v1/attribution/{app_user_id}` (design §10) — lets the customer's app show a user's
//! inviter (the attributed KOL). Read-only.
//!
//! **`evidence` is never returned here.** It is the sole evidence source for a KOL appeal and may
//! carry IPs / device IDs (§C2); the lookup path is a display convenience, not the appeal channel,
//! so the projection deliberately omits it. See `attribution::redeem` for where `evidence` is written.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::db;
use crate::models::AttributionMethod;

/// Public attribution view returned to the main app.
///
/// Mirrors `RedeemResult`'s shape: `attributed` is always present; the rest is omitted when the
/// user has no attribution — an organic user (no inviter) is a normal answer, not an error.
#[derive(Debug, Clone, Serialize)]
pub struct AttributionView {
    pub attributed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kol_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub campaign_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<AttributionMethod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<i16>,
    /// Frozen at attribution time — reflects the billing rules then in force, not today's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_billable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributed_at: Option<DateTime<Utc>>,
}

impl AttributionView {
    /// No attribution for this app user — a normal answer (organic users exist).
    fn none() -> Self {
        AttributionView {
            attributed: false,
            kol_id: None,
            campaign_id: None,
            link_id: None,
            method: None,
            confidence: None,
            is_billable: None,
            policy_version: None,
            attributed_at: None,
        }
    }
}

/// Projection of `attribution` — only the display fields, so `evidence` is never even fetched.
#[derive(sqlx::FromRow)]
struct Row {
    kol_id: i64,
    campaign_id: i64,
    link_id: i64,
    method: AttributionMethod,
    confidence: i16,
    is_billable: bool,
    policy_version: String,
    attributed_at: DateTime<Utc>,
}

impl Row {
    fn into_view(self) -> AttributionView {
        AttributionView {
            attributed: true,
            kol_id: Some(self.kol_id),
            campaign_id: Some(self.campaign_id),
            link_id: Some(self.link_id),
            method: Some(self.method),
            confidence: Some(self.confidence),
            is_billable: Some(self.is_billable),
            policy_version: Some(self.policy_version),
            attributed_at: Some(self.attributed_at),
        }
    }
}

/// Look up the attribution for an app user within a tenant.
///
/// Goes through `begin_tenant_tx` even though it only reads: querying tenant tables on the raw
/// pool skips the RLS context and fail-closes to empty rows (hard rule #4). The join to `player`
/// resolves `app_user_id` (bound at redeem) to its attribution; both tables are RLS-scoped, and
/// the unique `(tenant_id, app_user_id)` and `(tenant_id, player_id)` constraints make it single-row.
pub async fn lookup(
    pool: &PgPool,
    tenant_id: i64,
    app_user_id: &str,
) -> Result<AttributionView, sqlx::Error> {
    let mut tx = db::begin_tenant_tx(pool, tenant_id).await?;

    let found: Option<Row> = sqlx::query_as::<_, Row>(
        r#"
        SELECT a.kol_id, a.campaign_id, a.link_id, a.method, a.confidence,
               a.is_billable, a.policy_version, a.attributed_at
          FROM attribution a
          JOIN player p ON p.id = a.player_id
         WHERE p.app_user_id = $1
         LIMIT 1
        "#,
    )
    .bind(app_user_id)
    .fetch_optional(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(found
        .map(Row::into_view)
        .unwrap_or_else(AttributionView::none))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn sample_row() -> Row {
        Row {
            kol_id: 7,
            campaign_id: 3,
            link_id: 9,
            method: AttributionMethod::DeterministicCode,
            confidence: 100,
            is_billable: true,
            policy_version: "v1".to_string(),
            attributed_at: Utc.with_ymd_and_hms(2026, 7, 21, 12, 0, 0).unwrap(),
        }
    }

    /// An organic user must serialise to exactly `{"attributed": false}` — no null inviter fields
    /// the main app then has to special-case.
    #[test]
    fn none_serialises_to_just_attributed_false() {
        let j = serde_json::to_value(AttributionView::none()).unwrap();
        assert_eq!(j, serde_json::json!({ "attributed": false }));
    }

    #[test]
    fn populated_view_exposes_display_fields() {
        let j = serde_json::to_value(sample_row().into_view()).unwrap();
        assert_eq!(j["attributed"], serde_json::json!(true));
        assert_eq!(j["kol_id"], serde_json::json!(7));
        assert_eq!(j["campaign_id"], serde_json::json!(3));
        assert_eq!(j["link_id"], serde_json::json!(9));
        // Method serialises snake_case, matching the DB enum and the redeem response.
        assert_eq!(j["method"], serde_json::json!("deterministic_code"));
        assert_eq!(j["confidence"], serde_json::json!(100));
        assert_eq!(j["is_billable"], serde_json::json!(true));
        assert_eq!(j["policy_version"], serde_json::json!("v1"));
    }

    /// The lookup path must never leak appeal evidence or internal identifiers. If a future edit
    /// widens the projection, this fails before the data reaches a customer.
    #[test]
    fn view_never_exposes_evidence_or_internal_ids() {
        let j = serde_json::to_value(sample_row().into_view()).unwrap();
        for hidden in [
            "evidence",
            "tenant_id",
            "player_id",
            "id",
            "touch_at",
            "locked_until",
        ] {
            assert!(j.get(hidden).is_none(), "{hidden} must not be in the view");
        }
    }
}
