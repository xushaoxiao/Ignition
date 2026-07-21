//! Request-level identity verification.
//!
//! Every handler's first line lives here — they **no longer** read tenant from the body
//! or custom headers. Before this, the `X-Tenant-ID` header was the entire "auth":
//! anyone who guessed a tenant ID could redeem someone else's claim codes or inject
//! conversions into someone else's billing.

use std::sync::Arc;

use axum::http::{HeaderMap, Method, StatusCode, Uri};
use chrono::{DateTime, Utc};
use sqlx::Row;

use super::{ApiError, AppState};
use crate::auth::apikey::{self, Scope};
use crate::auth::jwt::{self, TokenKind};
use crate::auth::Caller;

fn unauthorized(msg: &str) -> ApiError {
    // Always the same code and wording: distinguishing "key missing" from "bad signature"
    // tells attackers which half they guessed correctly. Logs may differ; responses must not.
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        msg.to_string(),
        false,
    )
}

/// Verify an S2S caller (the customer's main-app backend).
///
/// The signature covers method + path + body, so `body` must be **raw bytes** — not
/// deserialise-then-reserialise JSON. Any key order or whitespace change breaks the
/// signature and surfaces as intermittent 401s for the customer.
pub async fn server_caller(
    state: &Arc<AppState>,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
    want: Scope,
    now: DateTime<Utc>,
) -> Result<Caller, ApiError> {
    let key_id = header(headers, apikey::HEADER_KEY)?;
    let timestamp = header(headers, apikey::HEADER_TIMESTAMP)?;
    let signature = header(headers, apikey::HEADER_SIGNATURE)?;

    let row = sqlx::query("SELECT id, tenant_id, secret_enc, scopes FROM auth_resolve_api_key($1)")
        .bind(key_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to resolve API key");
            ApiError::internal()
        })?
        .ok_or_else(|| unauthorized("Invalid credentials"))?;

    let api_key_id: i64 = row.try_get("id").map_err(|_| ApiError::internal())?;
    let tenant_id: i64 = row.try_get("tenant_id").map_err(|_| ApiError::internal())?;
    let secret_enc: Vec<u8> = row
        .try_get("secret_enc")
        .map_err(|_| ApiError::internal())?;
    let scopes: Vec<String> = row.try_get("scopes").map_err(|_| ApiError::internal())?;

    let secret = state.cipher.open(&secret_enc).map_err(|e| {
        // Log error type only — never ciphertext or key material.
        tracing::error!(error = %e, api_key_id, "failed to decrypt API key");
        ApiError::internal()
    })?;

    apikey::verify(
        secret.expose(),
        method.as_str(),
        uri.path(),
        body,
        timestamp,
        signature,
        apikey::DEFAULT_SKEW,
        now,
    )
    .map_err(|e| {
        tracing::info!(api_key_id, error = %e, "S2S signature verification failed");
        unauthorized("Invalid credentials")
    })?;

    if !apikey::has_scope(&scopes, want) {
        tracing::info!(api_key_id, want = want.as_str(), "key lacks required scope");
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden_scope",
            format!("This key does not have the {} scope", want.as_str()),
            false,
        ));
    }

    Ok(Caller {
        tenant_id,
        api_key_id,
    })
}

/// Verify the end user inside the TMA.
///
/// All IDs needed for attribution come from the token; the client must not supply them —
/// otherwise the frontend could set `kol_id` and attribute conversions to any KOL.
pub fn player_session(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    now: DateTime<Utc>,
) -> Result<jwt::Claims, ApiError> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| unauthorized("Missing session token"))?;

    state
        .issuer
        .verify(raw.trim(), TokenKind::Access, now)
        .map_err(|_| unauthorized("Session expired; please reopen the mini app"))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, ApiError> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| unauthorized("Missing signature headers"))
}
