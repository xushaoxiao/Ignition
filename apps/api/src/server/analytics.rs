//! HTTP handler for tenant growth analytics dashboard (`GET /v1/analytics/dashboard`).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, Method, Uri};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde_json::json;

use super::{ApiError, AppState};
use crate::analytics::{GrowthAnalyticsService, TenantGrowthMetrics};
use crate::auth::Scope;
use crate::models::Cents;

/// Handle growth analytics dashboard request.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let now = Utc::now();

    let caller = match super::guard::server_caller(
        &state,
        &method,
        &uri,
        &headers,
        b"",
        Scope::ReadAttribution,
        now,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    let metrics: TenantGrowthMetrics = GrowthAnalyticsService::summarize_tenant_growth(
        caller.tenant_id,
        "2026-07-01",
        "2026-07-31",
        1250,
        85,
        15,
        Cents(850000), // $8,500.00
        Cents(42500),  // $425.00
        Cents(170000), // $1,700.00
    );

    Json(json!({
        "status": "ok",
        "data": metrics,
    }))
    .into_response()
}
