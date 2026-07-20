//! 变现回传：主 App 把一笔 IAP 交易报给我们。
//!
//! **MVP 阶段这条链路只进分析流，不进账单。** 原因是结构性的：如果按客户回传
//! 的 IAP 计费，客户漏传一笔就少付我们一笔，而这种漏损几乎无法被发现 ——
//! 纯 take-rate 模式在 IAP 场景下有这个弱点。所以 MVP 的 CPA 建立在核销上
//! （我方可确认的事实），回传只用来算 LTV、验证 KOL 质量。
//!
//! 代码里的开关不是 `if MVP`，而是「`pricing_config.cpa_rates` 里配没配
//! `iap_purchase` 的单价」。等 CPA 验证完毕要开启 GMV 分成时，改的是一条定价
//! 配置，不是一次发版（约束 C4 的同一思路）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};

use super::policy::Policy;
use crate::db;
use crate::models::{event_type, BillableStatus, Cents};

/// 注意这里**没有**「无归因」这个错误变体。主 App 的绝大多数用户本来就不来自
/// 我们的渠道，查不到归因是正常情况：回传照收，只是 `attributed = false`。
/// 把它做成错误，客户端会当成失败去重试。
#[derive(Debug, thiserror::Error)]
pub enum PostbackError {
    #[error("attribution: 金额非法")]
    BadAmount,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// 一笔变现回传。
#[derive(Debug, Clone, Deserialize)]
pub struct Purchase {
    pub app_user_id: String,
    /// 主 App 侧的交易唯一 ID，幂等键。
    pub transaction_id: String,
    /// 金额，最小货币单位。
    pub amount: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub occurred_at: DateTime<Utc>,
}

fn default_currency() -> String {
    "USD".into()
}

/// 回传处理结果。
#[derive(Debug, Clone, Serialize)]
pub struct PostbackResult {
    /// 该用户是否归属于某个 KOL。
    pub attributed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kol_id: Option<i64>,
    /// 是否产生了可计费事件。MVP 下恒为 false —— 见模块文档。
    pub billable: bool,
    /// 命中幂等：这笔交易此前已经收到过。
    pub idempotent: bool,
}

/// 回传服务。
pub struct Service {
    pool: PgPool,
    policy: Policy,
}

impl Service {
    pub fn new(pool: PgPool, policy: Policy) -> Self {
        Service { pool, policy }
    }

    /// 记录一笔变现回传。
    pub async fn record(
        &self,
        tenant_id: i64,
        p: &Purchase,
        now: DateTime<Utc>,
    ) -> Result<PostbackResult, PostbackError> {
        if p.amount < 0 {
            return Err(PostbackError::BadAmount);
        }

        let mut tx = db::begin_tenant_tx(&self.pool, tenant_id).await?;

        // ---- 幂等：同一笔交易重复投递必然发生，一律返回首次结果 ----
        if let Some(row) = sqlx::query(
            "SELECT attribution_id, billable_event_id FROM purchase_event WHERE transaction_id = $1",
        )
        .bind(&p.transaction_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            let attribution_id: Option<i64> = row.try_get("attribution_id")?;
            let billable_event_id: Option<i64> = row.try_get("billable_event_id")?;
            let kol_id = match attribution_id {
                Some(id) => kol_of(&mut tx, id).await?,
                None => None,
            };
            tx.commit().await?;
            return Ok(PostbackResult {
                attributed: attribution_id.is_some(),
                kol_id,
                billable: billable_event_id.is_some(),
                idempotent: true,
            });
        }

        // ---- 归因查找 ----
        let attribution = sqlx::query(
            r#"
            SELECT a.id, a.kol_id, a.is_billable, a.locked_until
              FROM attribution a JOIN player pl ON pl.id = a.player_id
             WHERE pl.app_user_id = $1
            "#,
        )
        .bind(&p.app_user_id)
        .fetch_optional(&mut *tx)
        .await?;

        let (attribution_id, kol_id, is_billable) = match &attribution {
            Some(r) => (
                Some(r.try_get::<i64, _>("id")?),
                Some(r.try_get::<i64, _>("kol_id")?),
                r.try_get::<bool, _>("is_billable")?,
            ),
            None => (None, None, false),
        };

        // ---- 是否计费 ----
        // 三个条件缺一不可：有归因、该归因可计费（约束 C1）、且这个事件类型
        // 配了单价。第三条就是 GMV 分成的开关。
        let rate = self.cpa_rate(&mut tx, tenant_id, now).await?;
        let billable_event_id = match (attribution_id, is_billable, rate) {
            (Some(aid), true, Some(rate)) if rate.is_positive() => Some(
                self.insert_billable(&mut tx, tenant_id, aid, p, rate, now)
                    .await?,
            ),
            _ => None,
        };

        sqlx::query(
            r#"
            INSERT INTO purchase_event
              (tenant_id, attribution_id, app_user_id, transaction_id,
               amount_cents, currency, occurred_at, received_at, billable_event_id)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
            ON CONFLICT (tenant_id, transaction_id) DO NOTHING
            "#,
        )
        .bind(tenant_id)
        .bind(attribution_id)
        .bind(&p.app_user_id)
        .bind(&p.transaction_id)
        .bind(p.amount)
        .bind(&p.currency)
        .bind(p.occurred_at)
        .bind(now)
        .bind(billable_event_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(PostbackResult {
            attributed: attribution_id.is_some(),
            kol_id,
            billable: billable_event_id.is_some(),
            idempotent: false,
        })
    }

    /// 取 `iap_purchase` 的单价。没配就是没开启 GMV 分成。
    async fn cpa_rate(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: i64,
        now: DateTime<Utc>,
    ) -> Result<Option<Cents>, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT (cpa_rates->>$1)::bigint AS rate
              FROM pricing_config
             WHERE (tenant_id = $2 OR tenant_id IS NULL)
               AND effective_from <= $3
               AND (effective_to IS NULL OR effective_to > $3)
             -- 租户专属定价优先于全局默认
             ORDER BY tenant_id NULLS LAST, effective_from DESC
             LIMIT 1
            "#,
        )
        .bind(event_type::IAP_PURCHASE)
        .bind(tenant_id)
        .bind(now)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(row
            .and_then(|r| r.try_get::<Option<i64>, _>("rate").ok().flatten())
            .map(Cents))
    }

    async fn insert_billable(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        tenant_id: i64,
        attribution_id: i64,
        p: &Purchase,
        amount: Cents,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        // hold 期取 35 天，覆盖 App Store 的退款窗口 —— 否则钱付给 KOL 之后
        // 才发现是退款订单。
        let hold_until = p.occurred_at + self.policy.hold_period(event_type::IAP_PURCHASE);

        let row = sqlx::query(
            r#"
            INSERT INTO billable_event
              (tenant_id, attribution_id, event_type, external_id, status,
               amount_cents, currency, occurred_at, received_at, hold_until)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
            ON CONFLICT (tenant_id, event_type, external_id)
              DO UPDATE SET received_at = billable_event.received_at
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind(attribution_id)
        .bind(event_type::IAP_PURCHASE)
        .bind(&p.transaction_id)
        .bind(BillableStatus::Pending)
        .bind(amount.0)
        .bind(&p.currency)
        .bind(p.occurred_at)
        .bind(now)
        .bind(hold_until)
        .fetch_one(&mut **tx)
        .await?;
        row.try_get("id")
    }
}

async fn kol_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    attribution_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query("SELECT kol_id FROM attribution WHERE id = $1")
        .bind(attribution_id)
        .fetch_optional(&mut **tx)
        .await?;
    row.map(|r| r.try_get("kol_id")).transpose()
}
