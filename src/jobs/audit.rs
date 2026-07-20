//! 账本不变量的每日校验。
//!
//! 账本只可追加，所以一旦写进去一笔不平衡的分录，就没有「改回来」这个选项 ——
//! 只能靠更多的补偿分录去中和，而在那之前所有对账都是错的。
//!
//! 类型层面已经挡住了一半：`ledger::Txn` 不平衡就构造不出来。但类型管不到
//! 「有人绕过 Txn 直接写 SQL」和「一笔交易的分录只写进去一半就崩了」。
//! 这个任务负责另一半：**每天把库里的事实重新加一遍**。
//!
//! 失败就是告警，不是日志。账本对不上意味着已经开出去的账单可能是错的，
//! 这是这个系统里最严重的一类问题。

use serde::Serialize;
use sqlx::{PgPool, Row};

use crate::db;
use crate::models::Cents;

/// 一条不变量违规。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Violation {
    pub tenant_id: i64,
    pub rule: &'static str,
    pub detail: String,
}

/// 一次校验的输入：从库里聚合出来的事实。
#[derive(Debug, Default, Clone)]
pub struct Facts {
    /// 借贷不平的交易：(txn_id, 借方合计, 贷方合计)
    pub unbalanced_txns: Vec<(String, Cents, Cents)>,
    /// 全账本借方合计
    pub total_debit: Cents,
    /// 全账本贷方合计
    pub total_credit: Cents,
    /// `billed` 状态的可计费事件金额合计
    pub billed_events: Cents,
    /// `platform_revenue` 科目中**由可计费事件产生**的净额（贷 - 借）。
    ///
    /// 必须按 `ref_type` 过滤：平台订阅费也计入 `platform_revenue`，
    /// 但它不对应任何 `billable_event`。不过滤的话，每个有订阅费的租户
    /// 每月都会误报一次不平衡 —— 一个天天喊狼来了的告警等于没有告警。
    pub revenue_balance: Cents,
}

/// 校验不变量。纯函数：输入是事实，输出是违规列表，没有 IO。
///
/// 三条不变量对应设计文档 §3：
///
/// 1. 任一 `txn_id` 下 `sum(D) == sum(C)` —— 单笔交易平衡
/// 2. 全账本 `sum(D) == sum(C)` —— 总账平衡
/// 3. `billed` 事件金额合计 == `platform_revenue` 中来自可计费事件的净额
///    —— 账单与账本一致
///
/// 第 3 条最容易被误判为「多余」：如果 1 和 2 都成立，账本内部是自洽的。
/// 但自洽不等于正确 —— 一笔事件被标成 `billed` 却没写分录，或者写了分录却
/// 没标 `billed`，1 和 2 都发现不了，而这正是收入对不上的真实原因。
pub fn check(tenant_id: i64, f: &Facts) -> Vec<Violation> {
    let mut out = Vec::new();

    for (txn_id, debit, credit) in &f.unbalanced_txns {
        out.push(Violation {
            tenant_id,
            rule: "txn_balanced",
            detail: format!("交易 {txn_id} 借 {debit} / 贷 {credit}"),
        });
    }

    if f.total_debit != f.total_credit {
        out.push(Violation {
            tenant_id,
            rule: "ledger_balanced",
            detail: format!("全账本借 {} / 贷 {}", f.total_debit, f.total_credit),
        });
    }

    if f.billed_events != f.revenue_balance {
        out.push(Violation {
            tenant_id,
            rule: "revenue_matches_billed",
            detail: format!(
                "已开票事件合计 {} / platform_revenue 净额 {}",
                f.billed_events, f.revenue_balance
            ),
        });
    }

    out
}

/// 校验所有租户。返回全部违规；调用方负责告警。
pub async fn run(pool: &PgPool) -> anyhow::Result<Vec<Violation>> {
    let mut all = Vec::new();
    for tenant_id in super::all_tenant_ids(pool).await? {
        let facts = gather(pool, tenant_id).await?;
        let violations = check(tenant_id, &facts);
        if violations.is_empty() {
            tracing::info!(tenant_id, "账本校验通过");
        } else {
            for v in &violations {
                // error 级别：这条日志应当直接接告警，不要只留在日志里。
                tracing::error!(tenant_id, rule = v.rule, detail = %v.detail, "账本不变量被破坏");
            }
        }
        all.extend(violations);
    }
    Ok(all)
}

async fn gather(pool: &PgPool, tenant_id: i64) -> anyhow::Result<Facts> {
    let mut tx = db::begin_tenant_tx(pool, tenant_id).await?;

    let rows = sqlx::query(
        r#"
        SELECT txn_id::text AS txn_id,
               COALESCE(sum(amount_cents) FILTER (WHERE direction = 'D'), 0)::bigint AS debit,
               COALESCE(sum(amount_cents) FILTER (WHERE direction = 'C'), 0)::bigint AS credit
          FROM ledger_entry
         GROUP BY txn_id
        HAVING COALESCE(sum(amount_cents) FILTER (WHERE direction = 'D'), 0)
            <> COALESCE(sum(amount_cents) FILTER (WHERE direction = 'C'), 0)
        "#,
    )
    .fetch_all(&mut *tx)
    .await?;

    let unbalanced_txns = rows
        .into_iter()
        .map(|r| {
            Ok::<_, sqlx::Error>((
                r.try_get::<String, _>("txn_id")?,
                Cents(r.try_get::<i64, _>("debit")?),
                Cents(r.try_get::<i64, _>("credit")?),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let totals = sqlx::query(
        r#"
        -- 全部显式 ::bigint：Postgres 的 sum(bigint) 返回 NUMERIC，
        -- 不转换会在解码时报「i64 与 NUMERIC 不兼容」。
        SELECT COALESCE(sum(amount_cents) FILTER (WHERE direction = 'D'), 0)::bigint AS debit,
               COALESCE(sum(amount_cents) FILTER (WHERE direction = 'C'), 0)::bigint AS credit,
               -- ref_type 过滤：订阅费同样进 platform_revenue，但它不对应
               -- 任何 billable_event，混进来会让第三条不变量每月误报一次。
               (COALESCE(sum(amount_cents) FILTER (
                    WHERE account = 'platform_revenue' AND direction = 'C'
                      AND ref_type = 'billable_event'), 0)
              - COALESCE(sum(amount_cents) FILTER (
                    WHERE account = 'platform_revenue' AND direction = 'D'
                      AND ref_type = 'billable_event'), 0)
               )::bigint AS revenue
          FROM ledger_entry
        "#,
    )
    .fetch_one(&mut *tx)
    .await?;

    // 冲正过的事件状态是 reversed 而不是 billed，其分录在账本上被反向分录中和，
    // 因此两侧都自然排除掉了 —— 不需要在这里额外减去 credit。
    let billed = sqlx::query(
        "SELECT COALESCE(sum(amount_cents), 0)::bigint AS total FROM billable_event WHERE status = 'billed'",
    )
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(Facts {
        unbalanced_txns,
        total_debit: Cents(totals.try_get("debit")?),
        total_credit: Cents(totals.try_get("credit")?),
        billed_events: Cents(billed.try_get("total")?),
        revenue_balance: Cents(totals.try_get("revenue")?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn balanced() -> Facts {
        Facts {
            unbalanced_txns: vec![],
            total_debit: Cents(1000),
            total_credit: Cents(1000),
            billed_events: Cents(1000),
            revenue_balance: Cents(1000),
        }
    }

    #[test]
    fn a_healthy_ledger_produces_no_violations() {
        assert!(check(1, &balanced()).is_empty());
    }

    #[test]
    fn flags_an_unbalanced_transaction() {
        let f = Facts {
            unbalanced_txns: vec![("abc-123".into(), Cents(200), Cents(100))],
            ..balanced()
        };
        let v = check(1, &f);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].rule, "txn_balanced");
        assert!(v[0].detail.contains("abc-123"));
    }

    #[test]
    fn flags_a_global_imbalance() {
        let f = Facts {
            total_credit: Cents(900),
            ..balanced()
        };
        assert!(check(1, &f).iter().any(|v| v.rule == "ledger_balanced"));
    }

    /// 账本内部自洽但账单对不上：一笔事件被标成 billed 却没写分录。
    /// 前两条不变量都发现不了，而这正是收入对不上的真实成因。
    #[test]
    fn flags_billed_events_without_matching_revenue() {
        let f = Facts {
            billed_events: Cents(1200),
            ..balanced()
        };
        let v = check(1, &f);
        assert_eq!(v.len(), 1, "只应触发第三条不变量");
        assert_eq!(v[0].rule, "revenue_matches_billed");
    }

    #[test]
    fn reports_every_broken_invariant_not_just_the_first() {
        let f = Facts {
            unbalanced_txns: vec![("t1".into(), Cents(1), Cents(2))],
            total_debit: Cents(1),
            total_credit: Cents(2),
            billed_events: Cents(5),
            revenue_balance: Cents(6),
        };
        assert_eq!(check(1, &f).len(), 3, "应把三条问题一次报全");
    }

    /// 平台订阅费也进 `platform_revenue`，但它不对应任何可计费事件。
    /// `revenue_balance` 若把它算进来，每个有订阅的租户每月都会误报一次
    /// ——「账本不平」这种告警一旦有噪声就会被彻底忽略。
    #[test]
    fn subscription_revenue_is_not_compared_against_billed_events() {
        let f = Facts {
            // 账本总额含 9900 订阅费 + 200 CPA，但 revenue_balance 只取 CPA 那部分
            total_debit: Cents(10_100),
            total_credit: Cents(10_100),
            billed_events: Cents(200),
            revenue_balance: Cents(200),
            unbalanced_txns: vec![],
        };
        assert!(check(1, &f).is_empty(), "订阅费不该被算成账单差异");
    }

    #[test]
    fn an_empty_ledger_is_valid() {
        assert!(check(1, &Facts::default()).is_empty());
    }
}
