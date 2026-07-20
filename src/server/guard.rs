//! 请求级的身份校验。
//!
//! 所有 handler 的第一行都在这里 —— 它们**不再**从请求体或自定义头里读租户。
//! 在此之前，`X-Tenant-ID` 头就是全部的「认证」：任何人只要猜一个租户 ID
//! 就能核销别人的领奖码、往别人的账单里塞转化。

use std::sync::Arc;

use axum::http::{HeaderMap, Method, StatusCode, Uri};
use chrono::{DateTime, Utc};
use sqlx::Row;

use super::{ApiError, AppState};
use crate::auth::apikey::{self, Scope};
use crate::auth::jwt::{self, TokenKind};
use crate::auth::Caller;

fn unauthorized(msg: &str) -> ApiError {
    // 一律用同一个 code 与措辞：区分「密钥不存在」和「签名不对」，等于告诉
    // 攻击者哪一半猜对了。日志里可以细分，响应里不行。
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        msg.to_string(),
        false,
    )
}

/// 校验 S2S 调用方（客户主 App 的服务端）。
///
/// 签名覆盖 方法 + 路径 + 请求体，所以 `body` 必须是**原始字节**，
/// 不能是反序列化再序列化回去的结果 —— 那样 JSON 的键序或空白只要变一点，
/// 签名就对不上，表现为客户「随机地」收到 401。
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
            tracing::error!(error = %e, "解析 API Key 失败");
            ApiError::internal()
        })?
        .ok_or_else(|| unauthorized("凭据无效"))?;

    let api_key_id: i64 = row.try_get("id").map_err(|_| ApiError::internal())?;
    let tenant_id: i64 = row.try_get("tenant_id").map_err(|_| ApiError::internal())?;
    let secret_enc: Vec<u8> = row
        .try_get("secret_enc")
        .map_err(|_| ApiError::internal())?;
    let scopes: Vec<String> = row.try_get("scopes").map_err(|_| ApiError::internal())?;

    let secret = state.cipher.open(&secret_enc).map_err(|e| {
        // 只记错误类型，不记密文也不记密钥。
        tracing::error!(error = %e, api_key_id, "API Key 解密失败");
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
        tracing::info!(api_key_id, error = %e, "S2S 签名校验失败");
        unauthorized("凭据无效")
    })?;

    if !apikey::has_scope(&scopes, want) {
        tracing::info!(api_key_id, want = want.as_str(), "密钥无此权限");
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "forbidden_scope",
            format!("该密钥不具备 {} 权限", want.as_str()),
            false,
        ));
    }

    Ok(Caller {
        tenant_id,
        api_key_id,
    })
}

/// 校验 TMA 里的终端用户。
///
/// 归因所需的 ID 全部来自令牌，不接受客户端传入 —— 否则前端可以自己填
/// `kol_id`，把转化记到任意 KOL 名下。
pub fn player_session(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    now: DateTime<Utc>,
) -> Result<jwt::Claims, ApiError> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| unauthorized("缺少会话令牌"))?;

    state
        .issuer
        .verify(raw.trim(), TokenKind::Access, now)
        .map_err(|_| unauthorized("会话已失效，请重新打开小程序"))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, ApiError> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| unauthorized("缺少签名相关请求头"))
}
