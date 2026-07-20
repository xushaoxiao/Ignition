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

mod redeem;

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use sqlx::PgPool;

use crate::attribution;

pub struct AppState {
    pub pool: PgPool,
    pub attribution: attribution::Service,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/claims/redeem", post(redeem::handle))
        .with_state(state)
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
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self)).into_response()
    }
}
