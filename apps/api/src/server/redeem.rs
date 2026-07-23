//! Claim-code redemption endpoint.
//!
//! Critical path on the billing chain with higher SLO than the game path: if this is
//! down, the customer's users cannot collect rewards — it hurts the customer's customers.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;

use super::{client_ip, ApiError, AppState};
use crate::attribution::{RedeemError, RedeemRequest};
use crate::auth::Scope;

#[derive(Debug, Deserialize)]
pub struct Body {
    pub claim_code: String,
    pub app_user_id: String,
    #[serde(default)]
    pub device_id: Option<String>,
}

/// Take raw `Bytes`, not `Json<Body>`: the signature covers request-body bytes, so
/// verify on the raw payload before parsing. Reversing the order lets any JSON
/// normalisation (key order, whitespace, number format) cause intermittent 401s.
pub async fn handle(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
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
        Scope::Redeem,
        now,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    let body: Body = match serde_json::from_slice(&body) {
        Ok(b) => b,
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

    if body.claim_code.is_empty() || body.app_user_id.is_empty() {
        return ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "claim_code and app_user_id are required",
            false,
        )
        .into_response();
    }

    let req = RedeemRequest {
        tenant_id: caller.tenant_id,
        code: body.claim_code,
        app_user_id: body.app_user_id,
        device_id: body.device_id,
        ip: client_ip(&headers, peer),
        now,
    };

    match state.attribution.redeem(&req).await {
        Ok(res) => Json(res).into_response(),
        Err(e) => map_error(e).into_response(),
    }
}

/// Map domain errors to useful HTTP responses for the customer.
///
/// Only genuinely transient failures are marked `retryable`. Unknown, used, or expired
/// codes are terminal; retries only amplify useless traffic.
fn map_error(err: RedeemError) -> ApiError {
    use RedeemError::*;
    match err {
        Malformed => ApiError::new(
            StatusCode::BAD_REQUEST,
            "code_malformed",
            "Invalid claim code format",
            false,
        ),
        NotFound => ApiError::new(
            StatusCode::NOT_FOUND,
            "code_not_found",
            "Claim code not found",
            false,
        ),
        AlreadyUsed => ApiError::new(
            StatusCode::CONFLICT,
            "code_used",
            "Claim code already redeemed",
            false,
        ),
        Expired => ApiError::new(
            StatusCode::GONE,
            "code_expired",
            "Claim code expired",
            false,
        ),
        AlreadyBound => ApiError::new(
            StatusCode::CONFLICT,
            "already_bound",
            "This app user is already attributed to another channel",
            false,
        ),
        RiskDenied(rule) => {
            tracing::info!(rule, "redemption rejected by risk check");
            ApiError::new(
                StatusCode::FORBIDDEN,
                "risk_denied",
                "Risk check denied",
                false,
            )
        }
        Db(e) => {
            tracing::error!(error = %e, "redemption failed");
            ApiError::internal()
        }
    }
}
