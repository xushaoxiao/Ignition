//! Monetisation postback endpoint.
//!
//! The customer's main app reports an IAP transaction here. **In MVP this does not
//! produce billing** — see the module docs on `attribution::postback`.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;

use super::{ApiError, AppState};
use crate::attribution::postback::{PostbackError, Purchase};
use crate::auth::Scope;

/// Accept a monetisation postback.
///
/// Takes raw `Bytes`, not `Json<Purchase>`: the signature covers request-body bytes,
/// so verify on the raw payload before parsing. Doing it the other way round lets any
/// JSON normalisation (key order, whitespace, number format) break verification.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let now = Utc::now();

    let caller = match super::guard::server_caller(
        &state,
        &method,
        &uri,
        &headers,
        &body,
        Scope::Postback,
        now,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    let purchase: Purchase = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            return ApiError::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                format!("Invalid request body: {e}"),
                false,
            )
            .into_response()
        }
    };

    match state
        .postback
        .record(caller.tenant_id, &purchase, now)
        .await
    {
        Ok(res) => Json(res).into_response(),
        Err(PostbackError::BadAmount) => {
            ApiError::new(StatusCode::BAD_REQUEST, "bad_amount", "Amount must not be negative", false)
                .into_response()
        }
        Err(PostbackError::Db(e)) => {
            tracing::error!(error = %e, "postback processing failed");
            ApiError::internal().into_response()
        }
    }
}
