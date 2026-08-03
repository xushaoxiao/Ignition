//! Daily budget-decision game endpoints.
//!
//! ```text
//! GET  /v1/tma/daily              Today's scenario + the player's standing
//! POST /v1/tma/daily/answer       Record today's decision (idempotent per day)
//! GET  /v1/tma/daily/leaderboard  Campaign leaderboard
//! ```
//!
//! Same session guard as play and claim: every id comes from the token, never the body. A client
//! that could name its own `player_id` could answer on someone else's behalf and climb the
//! leaderboard without ever opening the game.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde::Deserialize;

use super::{ApiError, AppState};
use crate::daily::round::{DailyError, Player};

#[derive(Debug, Deserialize)]
pub struct AnswerBody {
    /// Key of the chosen option, from the scenario the client was served.
    pub choice_key: String,
}

pub async fn today(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let now = Utc::now();
    let claims = match super::guard::player_session(&state, &headers, now) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let req = Player::from_now(claims.tenant_id, claims.sub, claims.campaign_id, now);

    match state.daily.today(&req).await {
        Ok(view) => Json(view).into_response(),
        Err(e) => error_response(e),
    }
}

pub async fn answer(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AnswerBody>,
) -> Response {
    let now = Utc::now();
    let claims = match super::guard::player_session(&state, &headers, now) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let req = Player::from_now(claims.tenant_id, claims.sub, claims.campaign_id, now);

    match state.daily.answer(&req, &body.choice_key).await {
        Ok(res) => Json(res).into_response(),
        Err(e) => error_response(e),
    }
}

pub async fn leaderboard(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let now = Utc::now();
    let claims = match super::guard::player_session(&state, &headers, now) {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    let req = Player::from_now(claims.tenant_id, claims.sub, claims.campaign_id, now);

    match state.daily.leaderboard(&req).await {
        Ok(board) => Json(board).into_response(),
        Err(e) => error_response(e),
    }
}

fn error_response(e: DailyError) -> Response {
    match e {
        DailyError::CampaignInactive => ApiError::new(
            StatusCode::NOT_FOUND,
            "campaign_inactive",
            "Campaign is not active",
            false,
        )
        .into_response(),
        // A stale client sending an option we no longer serve. Retryable after a reload, so say so
        // rather than leaving the player tapping a dead button.
        DailyError::UnknownChoice => ApiError::new(
            StatusCode::BAD_REQUEST,
            "unknown_choice",
            "That option is no longer available; please reopen the mini app",
            false,
        )
        .into_response(),
        // Empty or malformed catalog is our deployment mistake, not the player's: generic message
        // outward, detail in the logs.
        DailyError::NoScenario | DailyError::Catalog(_) => {
            tracing::error!(error = %e, "daily scenario catalog is unusable");
            ApiError::internal().into_response()
        }
        DailyError::Db(e) => {
            tracing::error!(error = %e, "daily round failed");
            ApiError::internal().into_response()
        }
    }
}
