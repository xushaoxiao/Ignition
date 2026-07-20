//! 冷静期到期自动放行。
//!
//! `pending` 的事件在 `hold_until` 之后转入 `cleared`，成为可计入账单的转化。
//!
//! **`held` 不会被这个任务放行。** 风控暂缓的语义是「在人看过之前不收这笔钱」，
//! 让它随时间自动通过，等于把暂缓变成了一个延迟生效的放行 —— 那还不如不做。
//! 这条规则由 `billing::ready_to_clear` 保证，不在 SQL 里另写一遍条件：
//! 状态机只应该有一处实现。

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

use crate::models::{BillableEvent, BillableStatus};
use crate::{billing, db};

/// 一次运行的结果。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub scanned: usize,
    pub cleared: usize,
}

/// 放行所有租户中已过冷静期的事件。
pub async fn run(pool: &PgPool, now: DateTime<Utc>) -> anyhow::Result<Report> {
    let mut report = Report::default();
    for tenant_id in super::all_tenant_ids(pool).await? {
        let r = run_for_tenant(pool, tenant_id, now).await?;
        tracing::info!(
            tenant_id,
            scanned = r.scanned,
            cleared = r.cleared,
            "冷静期放行"
        );
        report.scanned += r.scanned;
        report.cleared += r.cleared;
    }
    Ok(report)
}

async fn run_for_tenant(
    pool: &PgPool,
    tenant_id: i64,
    now: DateTime<Utc>,
) -> anyhow::Result<Report> {
    let mut tx = db::begin_tenant_tx(pool, tenant_id).await?;

    // FOR UPDATE SKIP LOCKED：任务被重复拉起（调度器重试、人工重跑）时，
    // 两个实例不会争抢同一批行，也不会互相阻塞到超时。
    let rows = sqlx::query(
        r#"
        SELECT id, tenant_id, attribution_id, event_type, external_id, status,
               amount_cents, currency, over_cap, occurred_at, received_at,
               hold_until, cleared_at, billed_at, invoice_id, status_reason
          FROM billable_event
         WHERE status = 'pending' AND hold_until <= $1
         ORDER BY hold_until
         FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(now)
    .fetch_all(&mut *tx)
    .await?;

    let mut report = Report {
        scanned: rows.len(),
        cleared: 0,
    };

    for row in rows {
        let mut ev = BillableEvent::from_row(&row)?;

        // 状态机是唯一权威。SQL 的 WHERE 条件只是把候选集缩小，
        // 真正的判定在这里 —— 两处条件不一致时，以代码为准且必然被这里挡住。
        if !billing::ready_to_clear(&ev, now) {
            continue;
        }
        billing::transition(&mut ev, BillableStatus::Cleared, None, now)
            .map_err(|e| anyhow::anyhow!(e))?;

        sqlx::query(
            "UPDATE billable_event SET status = $2, cleared_at = $3 WHERE id = $1 AND status = 'pending'",
        )
        .bind(ev.id)
        .bind(ev.status)
        .bind(ev.cleared_at)
        .execute(&mut *tx)
        .await?;
        report.cleared += 1;
    }

    tx.commit().await?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{event_type, Cents};
    use chrono::{TimeDelta, TimeZone};

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, day, 0, 0, 0).unwrap()
    }

    fn event(status: BillableStatus, hold_until: DateTime<Utc>) -> BillableEvent {
        BillableEvent {
            id: 1,
            tenant_id: 1,
            attribution_id: 1,
            event_type: event_type::ACTIVATION.into(),
            external_id: "claim:1".into(),
            status,
            amount_cents: Cents(200),
            currency: "USD".into(),
            over_cap: false,
            occurred_at: at(1),
            received_at: at(1),
            hold_until,
            cleared_at: None,
            billed_at: None,
            invoice_id: None,
            status_reason: None,
        }
    }

    /// 这条测试守护的是「暂缓不会因为时间流逝而自动放行」。SQL 里已经写了
    /// `status = 'pending'`，这里再守一道 —— 那个条件很容易在改查询时被放宽。
    #[test]
    fn held_events_are_never_auto_cleared() {
        let ev = event(BillableStatus::Held, at(1));
        assert!(
            !billing::ready_to_clear(&ev, at(30)),
            "暂缓的事件被自动放行了"
        );
    }

    #[test]
    fn pending_clears_only_after_hold_expires() {
        let ev = event(BillableStatus::Pending, at(10));
        assert!(!billing::ready_to_clear(
            &ev,
            at(10) - TimeDelta::seconds(1)
        ));
        assert!(billing::ready_to_clear(&ev, at(10)));
        assert!(billing::ready_to_clear(&ev, at(11)));
    }
}
