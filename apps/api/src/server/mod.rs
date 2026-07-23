//! Public HTTP API.
//!
//! Integration cost directly affects SaaS sales friction, so the main app only needs
//! one required endpoint, plus two optional ones:
//!
//! ```text
//! POST /v1/claims/redeem            Redeem claim code (required)
//! POST /v1/postback/purchase        Monetisation postback (optional in MVP)
//! GET  /v1/attribution/{app_user}   Look up a user's inviter (optional)
//! ```
//!
//! Auth uses API Key + HMAC signature rather than OAuth — one fewer authorisation
//! round-trip means one fewer week on the customer's schedule.
//!
//! The TMA frontend has a separate `/v1/tma/*` surface using short-lived JWTs issued
//! after initData verification — fully separate from S2S credentials; the frontend
//! cannot hold long-lived secrets.

mod attribution_query;
mod guard;
mod postback;
mod redeem;
mod tma;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use sqlx::PgPool;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::attribution;
use crate::auth::jwt;
use crate::game;
use crate::secrets::Cipher;

pub struct AppState {
    pub pool: PgPool,
    /// Decrypts `_enc` fields — bot tokens and API key secrets are restored through it.
    pub cipher: Cipher,
    /// Issues TMA session tokens.
    pub issuer: jwt::Issuer,
    pub attribution: attribution::Service,
    pub issue: attribution::issue::Service,
    pub postback: attribution::postback::Service,
    pub game: game::play::Service,
}

/// Assemble routes.
///
/// When `cors_origins` is empty, no CORS layer is attached — S2S callers are server-side
/// and have no same-origin policy; only the TMA frontend needs CORS.
pub fn router(state: Arc<AppState>, cors_origins: &[String]) -> Router {
    let cors = cors_layer(cors_origins);
    let router = Router::new()
        .route("/healthz", get(health))
        // S2S: API Key + HMAC
        .route("/v1/claims/redeem", post(redeem::handle))
        .route("/v1/postback/purchase", post(postback::handle))
        .route(
            "/v1/attribution/{app_user_id}",
            get(attribution_query::handle),
        )
        // TMA: initData → JWT
        .route("/v1/tma/session", post(tma::session))
        .route("/v1/tma/session/refresh", post(tma::refresh))
        .route("/v1/tma/play", post(tma::play))
        .route("/v1/tma/claim", post(tma::claim))
        .with_state(state);

    match cors {
        Some(layer) => router.layer(layer),
        None => router,
    }
}

fn cors_layer(origins: &[String]) -> Option<CorsLayer> {
    if origins.is_empty() {
        return None;
    }
    let parsed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|o| match o.parse::<HeaderValue>() {
            Ok(v) => Some(v),
            Err(_) => {
                tracing::warn!(origin = %o, "invalid origin in http.cors_origins, ignored");
                None
            }
        })
        .collect();

    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(parsed))
            .allow_methods([Method::GET, Method::POST])
            // Allow only headers we actually use. Authorization is required (TMA session token),
            // but do not reach for `Any` — that would expose S2S signature headers to browsers.
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
    )
}

async fn health(axum::extract::State(state): axum::extract::State<Arc<AppState>>) -> Response {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(_) => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "db_unavailable",
            "Database unavailable",
            true,
        )
        .into_response(),
    }
}

/// Public error response.
///
/// `retryable` is deliberate: customers need to know whether to retry. Without that signal,
/// clients either retry blindly (amplifying outages) or give up blindly (losing revenue).
#[derive(Debug, Serialize)]
pub struct ApiError {
    #[serde(skip)]
    status: StatusCode,
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
    retryable: bool,
}

impl ApiError {
    pub fn new(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        ApiError {
            status,
            error: ErrorBody {
                code,
                message: message.into(),
                retryable,
            },
        }
    }

    /// Internal error. Externally always the same message — internal detail (which table,
    /// which key) goes to logs only, never the response.
    pub fn internal() -> Self {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal error",
            true,
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self)).into_response()
    }
}

/// Extract client IP.
///
/// In production, trust only headers injected by known reverse proxies; otherwise clients
/// can forge IP and bypass IP-based risk rate limits.
pub fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> Option<String> {
    if let Some(xff) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok())
        && let Some(first) = xff.split(',').next()
    {
        let ip = first.trim();
        if !ip.is_empty() {
            return Some(ip.to_string());
        }
    }
    Some(peer.ip().to_string())
}
