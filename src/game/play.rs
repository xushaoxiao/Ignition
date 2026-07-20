//! 一次抽奖的事务。

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};

use super::{draw, Prize};
use crate::db;
use crate::risk;

/// 库存竞争的重试次数。
///
/// 抽中的奖项可能在「读库存」与「扣库存」之间被别的请求抽空。扣减用
/// `UPDATE ... WHERE remaining > 0` 原子完成，抢输的一方拿到 0 行影响，
/// 重新读一次库存再抽。次数取 3：真实并发下抢输两次已经极罕见，再多就该
/// 怀疑是奖池确实空了，此时返回「已抽完」比无限重试更诚实。
const STOCK_RETRIES: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum PlayError {
    #[error("game: 活动不存在或未进行中")]
    CampaignInactive,
    #[error("game: 今日次数已用完")]
    RateLimited { limit: i64 },
    #[error("game: 奖池已抽完")]
    PoolExhausted,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// 一次抽奖请求。
#[derive(Debug, Clone)]
pub struct PlayRequest {
    pub tenant_id: i64,
    pub player_id: i64,
    pub campaign_id: i64,
    /// 客户端生成的幂等键。断网重试、用户狂点都会重复提交，没有它就会重复扣奖池。
    pub idempotency_key: String,
    pub ip: Option<String>,
    pub now: DateTime<Utc>,
}

/// 抽奖结果，回给 TMA 前端。
#[derive(Debug, Clone, Serialize)]
pub struct PlayResult {
    pub play_id: i64,
    pub prize_id: i64,
    pub prize_label: String,
    /// 该奖项在转盘上的下标，前端据此把指针停到对应扇区。
    pub segment_index: i32,
    pub plays_left: i64,
    /// 命中幂等：这次请求返回的是首次抽奖的结果。
    pub idempotent: bool,
}

/// 抽奖服务。
pub struct Service {
    pool: PgPool,
}

impl Service {
    pub fn new(pool: PgPool) -> Self {
        Service { pool }
    }

    /// 抽一次奖。
    ///
    /// 顺序刻意如此：先判幂等，再判频次，最后才动奖池。把幂等放在最前面，
    /// 重放请求就不会被频次规则误判为「又抽了一次」而拒绝 —— 那会让一次网络
    /// 重试表现为「今日次数已用完」，是最难解释的一类用户投诉。
    pub async fn play(&self, req: &PlayRequest) -> Result<PlayResult, PlayError> {
        let mut tx = db::begin_tenant_tx(&self.pool, req.tenant_id).await?;

        // ---- 幂等：重复提交返回首次结果，而不是报错 ----
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

        // ---- 活动状态 ----
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

        // ---- 风控 L1 ----
        // 抽奖侧可以直接拒绝：重来一次对真实用户没有损失，而每一次抽奖都在
        // 消耗真实的奖池成本。核销侧的取舍相反，见 risk::check_redeem。
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

        // ---- 抽奖 + 扣减库存 ----
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

        // ---- L2 信号：只存不判，这些数据事后无法补采 ----
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

    /// 抽一个奖并原子扣减其库存。
    ///
    /// 扣减用 `UPDATE ... WHERE remaining > 0 RETURNING` 一步完成，不做
    /// 「先查后改」—— 后者在并发下会把最后一件奖品发给两个人。
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
            // 抢输了：这一件在读与写之间被别人拿走。重读库存再抽。
        }
        Err(PlayError::PoolExhausted)
    }
}

/// 奖项按 id 升序返回 —— 前端转盘的扇区顺序必须稳定，否则同一个
/// `segment_index` 在两次请求里会指向不同扇区，指针停错位置。
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
