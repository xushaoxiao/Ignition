//! 定时任务。
//!
//! 三个任务，都用 `ignition <config> job <name>` 触发，由外部调度器（cron /
//! k8s CronJob）按时拉起。刻意不在服务进程里起后台循环：账单任务需要能被人手
//! 重跑、能指定账期、能单独看日志，塞进 HTTP 进程里这三件事都会变难。
//!
//! | 任务 | 频率 | 作用 |
//! |---|---|---|
//! | `clear-holds` | 每小时 | 冷静期到期的事件转入 `cleared` |
//! | `ledger-audit` | 每日 | 校验账本不变量，失败即告警 |
//! | `settle` | 每月 T+1 | 出账单：封顶、credit、复式分录 |

pub mod audit;
pub mod clear;
pub mod settle;

use sqlx::{PgPool, Row};

/// 列出所有租户 ID。
///
/// 批处理任务需要跨租户，而 `tenant` 表本身不带 RLS（它没有 `tenant_id` 可比）。
/// 逐个租户拿到 ID 后，具体的数据读写仍然走 `db::begin_tenant_tx`，
/// 每个租户一个事务 —— 一个租户的数据问题不会连累其他租户的账单。
pub async fn all_tenant_ids(pool: &PgPool) -> Result<Vec<i64>, sqlx::Error> {
    let rows = sqlx::query("SELECT id FROM tenant ORDER BY id")
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(|r| r.try_get("id")).collect()
}
