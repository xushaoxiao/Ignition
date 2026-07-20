//! 领奖码核销接口。
//!
//! 这是计费链路的关键路径，SLO 高于游戏链路：它挂了等于客户的用户领不到奖，
//! 损害的是客户的客户。

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

/// 取原始 `Bytes` 而非 `Json<Body>`：签名覆盖请求体的字节，必须先按原样
/// 验签再解析。顺序反了，任何 JSON 规范化（键序、空白、数字格式）都会让
/// 合法请求「随机地」401。
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
                format!("请求体格式非法: {e}"),
                false,
            )
            .into_response()
        }
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
            ApiError::internal()
        }
    }
}
