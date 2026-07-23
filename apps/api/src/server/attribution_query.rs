//! Attribution lookup endpoint (S2S).
//!
//! `GET /v1/attribution/{app_user_id}` — the customer's backend asks who invited a user, to show
//! the inviter in-app. Optional integration (design §10); read-only, never billing-affecting.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, Uri};
use axum::response::{IntoResponse, Response};
use chrono::Utc;

use super::{ApiError, AppState};
use crate::attribution::query;
use crate::auth::Scope;

/// Look up the attribution for one app user.
///
/// A GET carries no body, so the canonical signature covers `METHOD + path + empty body`. The
/// tenant comes from the verified API key, never the path — the path only names the app user to
/// look up, and RLS scopes the query to the caller's tenant regardless.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(app_user_id): Path<String>,
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

    match query::lookup(&state.pool, caller.tenant_id, &app_user_id).await {
        Ok(view) => Json(view).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "attribution lookup failed");
            ApiError::internal().into_response()
        }
    }
}
