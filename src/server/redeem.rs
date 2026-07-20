//! 领奖码核销接口。
//!
//! 这是计费链路的关键路径，SLO 高于游戏链路：它挂了等于客户的用户领不到奖，
//! 损害的是客户的客户。

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;

use super::{ApiError, AppState};
use crate::attribution::{RedeemError, RedeemRequest};

#[derive(Debug, Deserialize)]
pub struct Body {
    pub claim_code: String,
    pub app_user_id: String,
    #[serde(default)]
    pub device_id: Option<String>,
}

pub async fn handle(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<Body>,
) -> Response {
    let tenant_id = match auth_tenant(&headers) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    if body.claim_code.is_empty() || body.app_user_id.is_empty() {
        return ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "claim_code 与 app_user_id 必填",
            false,
        )
        .into_response();
    }

    let req = RedeemRequest {
        tenant_id,
        code: body.claim_code,
        app_user_id: body.app_user_id,
        device_id: body.device_id,
        ip: client_ip(&headers, peer),
        now: Utc::now(),
    };

    match state.attribution.redeem(&req).await {
        Ok(res) => Json(res).into_response(),
        Err(e) => map_error(e).into_response(),
    }
}

/// 把领域错误映射为对客户有用的 HTTP 响应。
///
/// 映射原则：只有真正瞬时的故障才标记 `retryable`。领奖码不存在、已用、过期
/// 都是终态，客户重试只会放大无效流量。
fn map_error(err: RedeemError) -> ApiError {
    use RedeemError::*;
    match err {
        Malformed => ApiError::new(
            StatusCode::BAD_REQUEST,
            "code_malformed",
            "领奖码格式非法",
            false,
        ),
        NotFound => ApiError::new(
            StatusCode::NOT_FOUND,
            "code_not_found",
            "领奖码不存在",
            false,
        ),
        AlreadyUsed => ApiError::new(StatusCode::CONFLICT, "code_used", "领奖码已被核销", false),
        Expired => ApiError::new(StatusCode::GONE, "code_expired", "领奖码已过期", false),
        AlreadyBound => ApiError::new(
            StatusCode::CONFLICT,
            "already_bound",
            "该 App 用户已归属其他渠道",
            false,
        ),
        RiskDenied(rule) => {
            tracing::info!(rule, "核销被风控拒绝");
            ApiError::new(StatusCode::FORBIDDEN, "risk_denied", "风控拒绝", false)
        }
        Db(e) => {
            tracing::error!(error = %e, "核销失败");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "内部错误",
                true,
            )
        }
    }
}

/// 解析并校验调用方身份。
///
/// **MVP 占位实现**：从 `X-Tenant-ID` 读取。上线前必须换成 API Key + HMAC 签名，
/// 否则任何人都能伪造租户身份核销任意领奖码。
///
/// TODO(auth): 接入 API Key 校验后移除对 X-Tenant-ID 的信任。
fn auth_tenant(headers: &HeaderMap) -> Result<i64, ApiError> {
    let unauthorized = |msg: &str| {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            msg.to_string(),
            false,
        )
    };

    let raw = headers
        .get("X-Tenant-ID")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| unauthorized("缺少租户标识"))?;

    match raw.parse::<i64>() {
        Ok(id) if id > 0 => Ok(id),
        _ => Err(unauthorized("租户标识非法")),
    }
}

/// 取客户端 IP。
///
/// 生产环境应改为只信任已知反代注入的头，否则客户端可以伪造 IP 绕过
/// 基于 IP 的风控限流。
fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> Option<String> {
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
