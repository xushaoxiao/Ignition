//! Telegram Mini App 的三个接口。
//!
//! ```text
//! POST /v1/tma/session    initData 换发会话令牌
//! POST /v1/tma/session/refresh
//! POST /v1/tma/play       抽奖（结果由服务端生成）
//! POST /v1/tma/claim      为一次抽奖签发领奖码
//! ```
//!
//! TMA 是 `ChannelAdapter` 这个扩展点的第一个实例。用它来验证抽象是否成立，
//! 而不是先写抽象再写实现 —— 只有一个实现的抽象，通常抽错了。

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::{client_ip, ApiError, AppState};
use crate::attribution::issue::{IssueError, IssueRequest};
use crate::auth::jwt::{SessionSubject, TokenKind};
use crate::entitlement::{self, ServiceLevel, SubscriptionStatus};
use crate::game::play::{PlayError, PlayRequest};
use crate::{db, telegram};

// ---------------------------------------------------------------- 开场

#[derive(Debug, Deserialize)]
pub struct SessionBody {
    /// `window.Telegram.WebApp.initData` 的原始字符串。
    ///
    /// 必须原样传，不要在前端解析后重新拼装 —— 签名是对原始字段序列算的。
    pub init_data: String,
}

#[derive(Debug, Serialize)]
pub struct SessionResponse {
    #[serde(flatten)]
    pub session: crate::auth::jwt::Session,
    pub campaign_id: i64,
    pub kol_id: i64,
    pub plays_left: i64,
    /// 转盘的扇区，**顺序即 `segment_index` 的取值顺序**。
    pub prizes: Vec<Segment>,
}

/// 一个转盘扇区。
///
/// 只给 id 和文案，**不给权重也不给库存**。权重泄漏等于把中奖概率公开，
/// 库存泄漏更糟 —— 用户能在转之前就看出限定奖已经没了，那这一次抽奖对他
/// 就毫无意义，他不会再往下走到领码那一步。
#[derive(Debug, Serialize)]
pub struct Segment {
    pub id: i64,
    pub label: String,
}

/// 无感登录：校验 initData，换发我们自己的短期令牌。
///
/// initData 校验通过后**不再复用它**。它是一段可被截获的静态字符串，
/// Telegram 不会主动使其失效，时效完全由我们把关（5 分钟）。让它承担后续
/// 每个请求的鉴权，等于把一个可长期重放的凭据发给了每一次网络中间环节。
pub async fn session(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SessionBody>,
) -> Response {
    let now = Utc::now();

    // start_param 就是 tracking_id。没有它就无法定位租户，也无法定位 KOL ——
    // 这种情况通常是用户直接打开了 Bot 而不是走 KOL 的分发链接。
    let Some(tracking_id) = telegram::start_param(&body.init_data) else {
        return ApiError::new(
            StatusCode::BAD_REQUEST,
            "missing_tracking_id",
            "缺少投放位标识，请从群内链接进入",
            false,
        )
        .into_response();
    };

    let row = match sqlx::query(
        r#"
        SELECT tenant_id, campaign_id, link_id, kol_id, bot_token_enc, sub_status, grace_until
          FROM auth_resolve_tracking($1)
        "#,
    )
    .bind(&tracking_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            return ApiError::new(
                StatusCode::NOT_FOUND,
                "campaign_not_found",
                "活动不存在或已结束",
                false,
            )
            .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "解析投放位失败");
            return ApiError::internal().into_response();
        }
    };

    let tenant_id: i64 = row.get("tenant_id");
    let campaign_id: i64 = row.get("campaign_id");
    let link_id: i64 = row.get("link_id");
    let kol_id: i64 = row.get("kol_id");
    let token_enc: Option<Vec<u8>> = row.get("bot_token_enc");
    let sub_status: Option<SubscriptionStatus> = row.get("sub_status");
    let grace_until: Option<chrono::DateTime<Utc>> = row.get("grace_until");

    // 订阅欠费超过宽限期后停止分发新会话。**宽限期内不停服** —— 断服损害的是
    // 客户的客户，客户的反应是流失而不是补款。
    let level = match sub_status {
        Some(s) => entitlement::service_level(s, grace_until, now),
        None => ServiceLevel::ReadOnly,
    };
    if level == ServiceLevel::ReadOnly {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            "campaign_suspended",
            "活动暂时不可用",
            false,
        )
        .into_response();
    }

    let Some(token_enc) = token_enc else {
        tracing::error!(tenant_id, "租户未配置 Bot token");
        return ApiError::internal().into_response();
    };
    let bot_token = match state.cipher.open(&token_enc) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(tenant_id, error = %e, "Bot token 解密失败");
            return ApiError::internal().into_response();
        }
    };
    let bot_token = match bot_token.expose_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            tracing::error!(tenant_id, "Bot token 不是合法 UTF-8");
            return ApiError::internal().into_response();
        }
    };

    let init = match telegram::verify(&body.init_data, &bot_token, telegram::DEFAULT_MAX_AGE, now) {
        Ok(d) => d,
        Err(e) => {
            // 不记 initData 原文：它带着可重放的签名。
            tracing::info!(tenant_id, error = %e, "initData 校验失败");
            return ApiError::new(
                StatusCode::UNAUTHORIZED,
                "bad_init_data",
                "登录信息无效，请重新打开小程序",
                false,
            )
            .into_response();
        }
    };

    let prizes = match load_segments(&state, tenant_id, campaign_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(tenant_id, error = %e, "读取奖池失败");
            return ApiError::internal().into_response();
        }
    };

    match upsert_player(&state, tenant_id, campaign_id, &init, now).await {
        Ok((player_id, plays_left)) => {
            let session = state.issuer.issue(
                &SessionSubject {
                    tenant_id,
                    player_id,
                    campaign_id,
                    link_id,
                    kol_id,
                },
                now,
            );
            Json(SessionResponse {
                session,
                campaign_id,
                kol_id,
                plays_left,
                prizes,
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!(tenant_id, error = %e, "创建玩家失败");
            ApiError::internal().into_response()
        }
    }
}

/// 读取转盘扇区。
///
/// `ORDER BY id` 必须与 `game::play` 里读奖池的顺序完全一致 —— 抽奖返回的
/// `segment_index` 是那个顺序下的下标。两边排序一旦不同，指针就会停在
/// 错误的扇区上：用户看到的奖和实际发的奖对不上，是最伤信任的一类 bug。
async fn load_segments(
    state: &Arc<AppState>,
    tenant_id: i64,
    campaign_id: i64,
) -> Result<Vec<Segment>, sqlx::Error> {
    let mut tx = db::begin_tenant_tx(&state.pool, tenant_id).await?;
    let rows = sqlx::query("SELECT id, label FROM reward_item WHERE campaign_id = $1 ORDER BY id")
        .bind(campaign_id)
        .fetch_all(&mut *tx)
        .await?;
    tx.commit().await?;

    rows.into_iter()
        .map(|r| {
            Ok(Segment {
                id: r.try_get("id")?,
                label: r.try_get("label")?,
            })
        })
        .collect()
}

async fn upsert_player(
    state: &Arc<AppState>,
    tenant_id: i64,
    campaign_id: i64,
    init: &telegram::InitData,
    now: chrono::DateTime<Utc>,
) -> Result<(i64, i64), sqlx::Error> {
    let mut tx = db::begin_tenant_tx(&state.pool, tenant_id).await?;

    // tg_username / premium 会变，每次进来刷新一遍 —— 它们是 L2 风控信号，
    // 事后无法补采。
    let row = sqlx::query(
        r#"
        INSERT INTO player (tenant_id, tg_user_id, tg_username, tg_is_premium, first_seen_at)
        VALUES ($1,$2,$3,$4,$5)
        ON CONFLICT (tenant_id, tg_user_id) DO UPDATE
           SET tg_username = EXCLUDED.tg_username,
               tg_is_premium = EXCLUDED.tg_is_premium
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(init.user.id)
    .bind(&init.user.username)
    .bind(init.user.is_premium)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    let player_id: i64 = row.try_get("id")?;

    // 开场就把剩余次数告诉前端，转盘才能在第一帧就正确地显示「今天还能抽几次」，
    // 而不是等用户点下去才被拒。
    let left = sqlx::query(
        r#"
        SELECT GREATEST(c.daily_play_limit - (
                 SELECT count(*) FROM game_play g
                  WHERE g.player_id = $1 AND g.campaign_id = c.id
                    AND g.created_at >= date_trunc('day', $3::timestamptz)
               ), 0) AS n
          FROM campaign c WHERE c.id = $2
        "#,
    )
    .bind(player_id)
    .bind(campaign_id)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?
    .try_get::<i64, _>("n")?;

    tx.commit().await?;
    Ok((player_id, left))
}

// ---------------------------------------------------------------- 刷新

#[derive(Debug, Deserialize)]
pub struct RefreshBody {
    pub refresh_token: String,
}

/// 用 refresh 令牌换一对新令牌。
///
/// 需要这个接口是因为 initData 在页面生命周期内不会更新，而用户可能把
/// Mini App 挂在后台超过 access 的 15 分钟。没有它，要么用户回来被登出，
/// 要么我们被迫把 access 的时效放长到几小时。
pub async fn refresh(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RefreshBody>,
) -> Response {
    let now = Utc::now();
    match state
        .issuer
        .verify(&body.refresh_token, TokenKind::Refresh, now)
    {
        Ok(c) => Json(state.issuer.issue(
            &SessionSubject {
                tenant_id: c.tenant_id,
                player_id: c.sub,
                campaign_id: c.campaign_id,
                link_id: c.link_id,
                kol_id: c.kol_id,
            },
            now,
        ))
        .into_response(),
        Err(_) => ApiError::new(
            StatusCode::UNAUTHORIZED,
            "session_expired",
            "会话已过期，请重新打开小程序",
            false,
        )
        .into_response(),
    }
}

// ---------------------------------------------------------------- 抽奖

#[derive(Debug, Deserialize)]
pub struct PlayBody {
    /// 客户端生成的 UUID。同一次点击的重试必须带同一个值。
    pub idempotency_key: String,
}

pub async fn play(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PlayBody>,
) -> Response {
    let now = Utc::now();
    let claims = match super::guard::player_session(&state, &headers, now) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    if body.idempotency_key.is_empty() {
        return ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "idempotency_key 必填",
            false,
        )
        .into_response();
    }

    let req = PlayRequest {
        tenant_id: claims.tenant_id,
        player_id: claims.sub,
        campaign_id: claims.campaign_id,
        idempotency_key: body.idempotency_key,
        ip: client_ip(&headers, peer),
        now,
    };

    match state.game.play(&req).await {
        Ok(res) => Json(res).into_response(),
        Err(PlayError::CampaignInactive) => ApiError::new(
            StatusCode::NOT_FOUND,
            "campaign_inactive",
            "活动未在进行中",
            false,
        )
        .into_response(),
        Err(PlayError::RateLimited { limit }) => ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "daily_limit_reached",
            format!("今日已抽满 {limit} 次，明天再来"),
            false,
        )
        .into_response(),
        Err(PlayError::PoolExhausted) => ApiError::new(
            StatusCode::CONFLICT,
            "pool_exhausted",
            "奖品已被领完",
            false,
        )
        .into_response(),
        Err(PlayError::Db(e)) => {
            tracing::error!(error = %e, "抽奖失败");
            ApiError::internal().into_response()
        }
    }
}

// ---------------------------------------------------------------- 领奖码

#[derive(Debug, Deserialize)]
pub struct ClaimBody {
    pub play_id: i64,
}

pub async fn claim(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ClaimBody>,
) -> Response {
    let now = Utc::now();
    let claims = match super::guard::player_session(&state, &headers, now) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    let req = IssueRequest {
        tenant_id: claims.tenant_id,
        player_id: claims.sub,
        campaign_id: claims.campaign_id,
        link_id: claims.link_id,
        kol_id: claims.kol_id,
        game_play_id: body.play_id,
        now,
    };

    match state.issue.issue(&req).await {
        Ok(res) => Json(res).into_response(),
        Err(IssueError::PlayNotFound) => ApiError::new(
            StatusCode::NOT_FOUND,
            "play_not_found",
            "抽奖记录不存在",
            false,
        )
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "签发领奖码失败");
            ApiError::internal().into_response()
        }
    }
}
