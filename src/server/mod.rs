//! 对外 HTTP 接口。
//!
//! 接入成本直接决定 SaaS 的销售阻力，所以对主 App 只暴露两个必接接口：
//!
//! ```text
//! POST /v1/claims/redeem       核销领奖码（必接）
//! POST /v1/postback/purchase   变现回传（可选，MVP 可后接）
//! ```
//!
//! 认证用 API Key + HMAC 签名而非 OAuth —— 少一轮授权流程，就少一周的客户排期。
//!
//! TMA 前端另有一组 `/v1/tma/*` 接口，走 initData 换发的短期 JWT，
//! 与 S2S 那套凭据完全分开：前端保不住长期密钥。

mod guard;
mod postback;
mod redeem;
mod tma;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
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
    /// `_enc` 字段的解密器。Bot token 与 API Key secret 都经它还原。
    pub cipher: Cipher,
    /// TMA 会话令牌的签发者。
    pub issuer: jwt::Issuer,
    pub attribution: attribution::Service,
    pub issue: attribution::issue::Service,
    pub postback: attribution::postback::Service,
    pub game: game::play::Service,
}

/// 组装路由。
///
/// `cors_origins` 为空时不挂 CORS 层 —— S2S 调用来自服务端，没有同源策略，
/// 只有 TMA 前端需要它。
pub fn router(state: Arc<AppState>, cors_origins: &[String]) -> Router {
    let cors = cors_layer(cors_origins);
    let router = Router::new()
        .route("/healthz", get(health))
        // S2S：API Key + HMAC
        .route("/v1/claims/redeem", post(redeem::handle))
        .route("/v1/postback/purchase", post(postback::handle))
        // TMA：initData → JWT
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
                tracing::warn!(origin = %o, "http.cors_origins 中有非法来源，已忽略");
                None
            }
        })
        .collect();

    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(parsed))
            .allow_methods([Method::GET, Method::POST])
            // 只放行实际用到的头。Authorization 是必须的（TMA 的会话令牌），
            // 但不要顺手加上 `Any` —— 那会连 S2S 的签名头一起对浏览器放开。
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]),
    )
}

async fn health(axum::extract::State(state): axum::extract::State<Arc<AppState>>) -> Response {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(_) => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "db_unavailable",
            "数据库不可用",
            true,
        )
        .into_response(),
    }
}

/// 对外错误响应。
///
/// `retryable` 是刻意设计的：客户需要知道一个错误该不该重试。不给这个信号，
/// 客户端要么无脑重试（放大故障），要么无脑放弃（丢收入）。
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

    /// 内部错误。对外一律是同一句话 —— 内部细节（哪张表、哪把密钥）
    /// 只进日志，不进响应。
    pub fn internal() -> Self {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "内部错误",
            true,
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self)).into_response()
    }
}

/// 取客户端 IP。
///
/// 生产环境应改为只信任已知反代注入的头，否则客户端可以伪造 IP 绕过
/// 基于 IP 的风控限流。
pub fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> Option<String> {
    if let Some(xff) = headers.get("X-Forwarded-For").and_then(|v| v.to_str().ok()) {
        if let Some(first) = xff.split(',').next() {
            let ip = first.trim();
            if !ip.is_empty() {
                return Some(ip.to_string());
            }
        }
    }
    Some(peer.ip().to_string())
}
