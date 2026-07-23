//! One play transaction.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};

use super::{Prize, draw};
use crate::db;
use crate::risk;

/// Retries when competing for prize stock.
///
/// A drawn prize may sell out between read and decrement. Decrement uses
/// `UPDATE ... WHERE remaining > 0` atomically; losers get zero rows affected and re-read stock.
/// Three retries: losing twice under real concurrency is already rare; more suggests the pool is
/// actually empty — "sold out" is more honest than infinite retry.
const STOCK_RETRIES: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum PlayError {
    #[error("game: campaign not found or not active")]
    CampaignInactive,
    #[error("game: daily play limit reached")]
    RateLimited { limit: i64 },
    #[error("game: prize pool exhausted")]
    PoolExhausted,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// One play request.
#[derive(Debug, Clone)]
pub struct PlayRequest {
    pub tenant_id: i64,
    pub player_id: i64,
    pub campaign_id: i64,
    /// Client-generated idempotency key. Retries and double-clicks must reuse the same value.
    pub idempotency_key: String,
    pub ip: Option<String>,
    pub now: DateTime<Utc>,
}

/// Play result returned to the TMA frontend.
#[derive(Debug, Clone, Serialize)]
pub struct PlayResult {
    pub play_id: i64,
    pub prize_id: i64,
    pub prize_label: String,
    /// Index of this prize on the wheel; frontend stops the pointer on this segment.
    pub segment_index: i32,
    pub plays_left: i64,
    /// Idempotent hit: this response repeats the first play outcome.
    pub idempotent: bool,
}

/// Play service.
pub struct Service {
    pool: PgPool,
}

impl Service {
    pub fn new(pool: PgPool) -> Self {
        Service { pool }
    }

    /// Execute one play.
    ///
    /// Order is deliberate: idempotency first, then rate limit, then prize pool. Idempotency first
    /// so replays are not rejected as "daily limit reached" — the hardest user complaint to explain
    /// after a network retry.
    pub async fn play(&self, req: &PlayRequest) -> Result<PlayResult, PlayError> {
        let mut tx = db::begin_tenant_tx(&self.pool, req.tenant_id).await?;

        // ---- idempotency: duplicate submit returns first outcome, not an error ----
        let existing = sqlx::query(
            r#"
            SELECT gp.id, gp.reward_item_id, gp.segment_index, ri.label
              FROM game_play gp
              JOIN reward_item ri ON ri.id = gp.reward_item_id
             WHERE gp.player_id = $1 AND gp.campaign_id = $2 AND gp.idempotency_key = $3
            "#,
        )
        .bind(req.player_id)
        .bind(req.campaign_id)
        .bind(&req.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(row) = existing {
            let plays_left = plays_left(&mut tx, req).await?;
            tx.commit().await?;
            return Ok(PlayResult {
                play_id: row.try_get("id")?,
                prize_id: row.try_get("reward_item_id")?,
                prize_label: row.try_get("label")?,
                segment_index: row.try_get("segment_index")?,
                plays_left,
                idempotent: true,
            });
        }

        // ---- campaign status ----
        let campaign = sqlx::query(
            r#"
            SELECT daily_play_limit
              FROM campaign
             WHERE id = $1 AND status = 'active'
               AND (starts_at IS NULL OR starts_at <= $2)
               AND (ends_at   IS NULL OR ends_at   >  $2)
            "#,
        )
        .bind(req.campaign_id)
        .bind(req.now)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PlayError::CampaignInactive)?;
        let daily_play_limit: i32 = campaign.try_get("daily_play_limit")?;

        // ---- risk L1 ----
        // Play can deny outright: retry is free for real users, and every play consumes real prize cost.
        // Redemption is the opposite trade-off — see risk::check_redeem.
        let today_play_count = today_play_count(&mut tx, req).await?;
        let verdict = risk::check_play(&risk::PlayInput {
            today_play_count,
            daily_play_limit: daily_play_limit as i64,
        });
        if verdict.action == risk::Action::Deny {
            return Err(PlayError::RateLimited {
                limit: daily_play_limit as i64,
            });
        }

        // ---- draw + stock decrement ----
        let (prize, segment_index) = self.draw_and_reserve(&mut tx, req).await?;

        let play = sqlx::query(
            r#"
            INSERT INTO game_play
              (tenant_id, player_id, campaign_id, idempotency_key,
               reward_item_id, segment_index, result, created_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
            RETURNING id
            "#,
        )
        .bind(req.tenant_id)
        .bind(req.player_id)
        .bind(req.campaign_id)
        .bind(&req.idempotency_key)
        .bind(prize.id)
        .bind(segment_index)
        .bind(serde_json::json!({ "label": prize.label, "weight": prize.weight }))
        .bind(req.now)
        .fetch_one(&mut *tx)
        .await?;
        let play_id: i64 = play.try_get("id")?;

        // ---- L2 signals: store only, decide later — cannot backfill ----
        sqlx::query(
            r#"
            INSERT INTO risk_signal (tenant_id, player_id, stage, ip, attrs, created_at)
            VALUES ($1,$2,'play', NULLIF($3,'')::inet, $4, $5)
            "#,
        )
        .bind(req.tenant_id)
        .bind(req.player_id)
        .bind(req.ip.clone().unwrap_or_default())
        .bind(serde_json::json!({ "campaign_id": req.campaign_id, "prize_id": prize.id }))
        .bind(req.now)
        .execute(&mut *tx)
        .await?;

        let plays_left = (daily_play_limit as i64 - today_play_count - 1).max(0);
        tx.commit().await?;

        Ok(PlayResult {
            play_id,
            prize_id: prize.id,
            prize_label: prize.label,
            segment_index,
            plays_left,
            idempotent: false,
        })
    }

    /// Draw a prize and atomically decrement its stock.
    ///
    /// Decrement uses `UPDATE ... WHERE remaining > 0 RETURNING` in one step — not read-then-write,
    /// which can award the last unit to two concurrent requests.
    async fn draw_and_reserve(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        req: &PlayRequest,
    ) -> Result<(Prize, i32), PlayError> {
        for _ in 0..STOCK_RETRIES {
            let prizes = load_prizes(tx, req.campaign_id).await?;
            let Some(idx) = draw(&prizes) else {
                return Err(PlayError::PoolExhausted);
            };
            let prize = &prizes[idx];

            let taken = sqlx::query(
                r#"
                UPDATE reward_item
                   SET remaining = remaining - 1, version = version + 1
                 WHERE id = $1 AND remaining > 0
                RETURNING id
                "#,
            )
            .bind(prize.id)
            .fetch_optional(&mut **tx)
            .await?;

            if taken.is_some() {
                return Ok((prize.clone(), idx as i32));
            }
            // Lost the race: item gone between read and write. Re-read stock and retry.
        }
        Err(PlayError::PoolExhausted)
    }
}

/// Prizes returned in ascending id order — frontend wheel segment order must be stable or the same
/// `segment_index` points at different segments across requests.
async fn load_prizes(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    campaign_id: i64,
) -> Result<Vec<Prize>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, label, weight, remaining FROM reward_item WHERE campaign_id = $1 ORDER BY id",
    )
    .bind(campaign_id)
    .fetch_all(&mut **tx)
    .await?;

    rows.into_iter()
        .map(|r| {
            Ok(Prize {
                id: r.try_get("id")?,
                label: r.try_get("label")?,
                weight: r.try_get::<i32, _>("weight")? as i64,
                remaining: r.try_get("remaining")?,
            })
        })
        .collect()
}

async fn today_play_count(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    req: &PlayRequest,
) -> Result<i64, sqlx::Error> {
    sqlx::query(
        r#"
        SELECT count(*) AS n FROM game_play
         WHERE player_id = $1 AND campaign_id = $2
           AND created_at >= date_trunc('day', $3::timestamptz)
        "#,
    )
    .bind(req.player_id)
    .bind(req.campaign_id)
    .bind(req.now)
    .fetch_one(&mut **tx)
    .await?
    .try_get("n")
}

async fn plays_left(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    req: &PlayRequest,
) -> Result<i64, sqlx::Error> {
    let limit: i32 = sqlx::query("SELECT daily_play_limit FROM campaign WHERE id = $1")
        .bind(req.campaign_id)
        .fetch_one(&mut **tx)
        .await?
        .try_get("daily_play_limit")?;
    let used = today_play_count(tx, req).await?;
    Ok((limit as i64 - used).max(0))
}
