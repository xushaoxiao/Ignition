//! 变现回传接口。
//!
//! 客户主 App 在这里把一笔 IAP 交易报给我们。**MVP 阶段它不产生账单**
//! —— 见 `attribution::postback` 的模块文档。

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

/// 接收一笔变现回传。
///
/// 取的是原始 `Bytes` 而不是 `Json<Purchase>`：签名覆盖请求体的字节，
/// 必须先按原样验签再解析。反过来做的话，任何 JSON 规范化（键序、空白、
/// 数字格式）都会让合法请求验签失败。
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
                format!("请求体格式非法: {e}"),
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
            ApiError::new(StatusCode::BAD_REQUEST, "bad_amount", "金额不能为负", false)
                .into_response()
        }
        Err(PostbackError::Db(e)) => {
            tracing::error!(error = %e, "回传处理失败");
            ApiError::internal().into_response()
        }
    }
}
