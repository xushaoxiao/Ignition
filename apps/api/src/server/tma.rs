//! Telegram Mini App HTTP endpoints.
//!
//! ```text
//! POST /v1/tma/session           Exchange initData for session tokens
//! POST /v1/tma/session/refresh
//! POST /v1/tma/play              Play (outcome generated server-side)
//! POST /v1/tma/claim             Issue a claim code for a play
//! ```
//!
//! TMA is the first instance of the `ChannelAdapter` extension point. Use it to validate
//! the abstraction — not the other way around. An abstraction with one implementation is
//! usually the wrong abstraction.

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

// ---------------------------------------------------------------- session

#[derive(Debug, Deserialize)]
pub struct SessionBody {
    /// Raw `window.Telegram.WebApp.initData` string.
    ///
    /// Must be sent verbatim — do not parse and reassemble on the frontend; the signature
    /// is over the original field sequence.
    pub init_data: String,
}

#[derive(Debug, Serialize)]
pub struct SessionResponse {
    #[serde(flatten)]
    pub session: crate::auth::jwt::Session,
    pub campaign_id: i64,
    pub kol_id: i64,
    pub plays_left: i64,
    /// Wheel segments; **order defines valid `segment_index` values**.
    pub prizes: Vec<Segment>,
}

/// One wheel segment.
///
/// Exposes id and label only — **not weight or stock**. Leaking weight reveals win odds;
/// leaking stock is worse — users can see a limited prize is gone before spinning, making
/// the spin pointless and stopping them before claim-code issuance.
#[derive(Debug, Serialize)]
pub struct Segment {
    pub id: i64,
    pub label: String,
}

/// Silent login: verify initData, issue our own short-lived tokens.
///
/// **Do not reuse initData after verification.** It is a static, interceptable string;
/// Telegram does not invalidate it proactively; freshness is entirely ours (5 minutes).
/// Using it for every subsequent request hands a long-replayable credential to every network hop.
pub async fn session(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SessionBody>,
) -> Response {
    let now = Utc::now();

    // start_param is tracking_id. Without it we cannot locate tenant or KOL — usually the user
    // opened the bot directly instead of a KOL distribution link.
    let Some(tracking_id) = telegram::start_param(&body.init_data) else {
        return ApiError::new(
            StatusCode::BAD_REQUEST,
            "missing_tracking_id",
            "Missing placement identifier; please enter via the group link",
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
                "Campaign not found or has ended",
                false,
            )
            .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to resolve tracking placement");
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

    // Stop issuing new sessions after subscription is past_due beyond grace. **No outage during
    // grace** — cutting service hurts the customer's customers; churn beats payment recovery.
    let level = match sub_status {
        Some(s) => entitlement::service_level(s, grace_until, now),
        None => ServiceLevel::ReadOnly,
    };
    if level == ServiceLevel::ReadOnly {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            "campaign_suspended",
            "Campaign temporarily unavailable",
            false,
        )
        .into_response();
    }

    let Some(token_enc) = token_enc else {
        tracing::error!(tenant_id, "tenant has no bot token configured");
        return ApiError::internal().into_response();
    };
    let bot_token = match state.cipher.open(&token_enc) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(tenant_id, error = %e, "failed to decrypt bot token");
            return ApiError::internal().into_response();
        }
    };
    let bot_token = match bot_token.expose_str() {
        Ok(s) => s.to_string(),
        Err(_) => {
            tracing::error!(tenant_id, "bot token is not valid UTF-8");
            return ApiError::internal().into_response();
        }
    };

    let init = match telegram::verify(&body.init_data, &bot_token, telegram::DEFAULT_MAX_AGE, now) {
        Ok(d) => d,
        Err(e) => {
            // Do not log raw initData — it carries a replayable signature.
            tracing::info!(tenant_id, error = %e, "initData verification failed");
            return ApiError::new(
                StatusCode::UNAUTHORIZED,
                "bad_init_data",
                "Invalid login data; please reopen the mini app",
                false,
            )
            .into_response();
        }
    };

    let prizes = match load_segments(&state, tenant_id, campaign_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(tenant_id, error = %e, "failed to load prize pool");
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
            tracing::error!(tenant_id, error = %e, "failed to create player");
            ApiError::internal().into_response()
        }
    }
}

/// Load wheel segments.
///
/// `ORDER BY id` must match the prize order in `game::play` — the play response's
/// `segment_index` is an index in that order. Diverging sort orders land the pointer on the
/// wrong segment: what the user sees and what was awarded diverge — one of the worst trust bugs.
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

    // tg_username / premium change over time — refresh on every entry; they are L2 risk
    // signals that cannot be backfilled later.
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

    // Tell the frontend remaining plays at session start so the wheel can show "plays left
    // today" on the first frame, not only after the user clicks and gets rejected.
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

// ---------------------------------------------------------------- refresh

#[derive(Debug, Deserialize)]
pub struct RefreshBody {
    pub refresh_token: String,
}

/// Exchange a refresh token for a fresh token pair.
///
/// Needed because initData does not update during the page lifecycle, yet users may leave the
/// mini app in the background longer than the 15-minute access TTL. Without this, they return
/// logged out — or we must lengthen access to hours, widening token leak impact.
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
            "Session expired; please reopen the mini app",
            false,
        )
        .into_response(),
    }
}

// ---------------------------------------------------------------- play

#[derive(Debug, Deserialize)]
pub struct PlayBody {
    /// Client-generated UUID. Retries for the same click must reuse the same value.
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
            "idempotency_key is required",
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
            "Campaign is not active",
            false,
        )
        .into_response(),
        Err(PlayError::RateLimited { limit }) => ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "daily_limit_reached",
            format!("Daily limit of {limit} plays reached; try again tomorrow"),
            false,
        )
        .into_response(),
        Err(PlayError::PoolExhausted) => ApiError::new(
            StatusCode::CONFLICT,
            "pool_exhausted",
            "All prizes have been claimed",
            false,
        )
        .into_response(),
        Err(PlayError::Db(e)) => {
            tracing::error!(error = %e, "play failed");
            ApiError::internal().into_response()
        }
    }
}

// ---------------------------------------------------------------- claim code

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
            "Play record not found",
            false,
        )
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to issue claim code");
            ApiError::internal().into_response()
        }
    }
}
