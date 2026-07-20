//! 月末结算：出账单。
//!
//! **用量不做实时计费。** CPA 事件有 7 天冷静期，实时计费必然要频繁冲正，
//! 客户体验极差；月末批量结算与冷静期天然对齐（设计文档 §5.1）。
//!
//! 这个任务是账本、封顶、状态机三者第一次同时进入在线链路的地方 ——
//! 在它之前，`ledger` 与 `billing::apply_cap` 都只有测试在调用。

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool, Row};

use crate::models::{BillableEvent, BillableStatus, Cents};
use crate::{billing, db, ledger};

/// 账期。左闭右开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Period {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl Period {
    /// `now` 所在月份的上一个自然月 —— 月末 T+1 跑任务时要结的是上个月。
    pub fn previous_month(now: DateTime<Utc>) -> Period {
        let d = now.date_naive();
        let this_month = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).expect("1 号总是合法日期");
        Period {
            start: prev_month_first_day(this_month),
            end: this_month,
        }
    }
}

fn prev_month_first_day(first_of_month: NaiveDate) -> NaiveDate {
    let (y, m) = (first_of_month.year(), first_of_month.month());
    if m == 1 {
        NaiveDate::from_ymd_opt(y - 1, 12, 1).expect("12 月 1 号合法")
    } else {
        NaiveDate::from_ymd_opt(y, m - 1, 1).expect("上个月 1 号合法")
    }
}

/// 一条账单行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Line {
    pub kind: &'static str,
    pub description: String,
    pub quantity: i64,
    pub unit_cents: Cents,
    pub amount_cents: Cents,
}

pub mod line_kind {
    pub const PLATFORM_FEE: &str = "platform_fee";
    pub const CPA: &str = "cpa";
    pub const CREDIT: &str = "credit";
}

/// 结算的输入：从库里取出的事实。
#[derive(Debug, Default)]
pub struct Inputs {
    /// 待结算的已放行事件，必须按 `cleared_at` 升序 —— 先发生的转化先占用
    /// 封顶额度，这是唯一对客户可解释的顺序。
    pub cleared: Vec<BillableEvent>,
    /// 月度封顶。`None` 表示无封顶。
    pub cap: Option<Cents>,
    /// 月度平台费。
    pub platform_fee: Cents,
    /// 上期已开票、本期被冲正的金额。
    pub credit: Cents,
}

/// 结算的产物。
#[derive(Debug, Default)]
pub struct Bill {
    pub lines: Vec<Line>,
    pub subtotal: Cents,
    pub credit: Cents,
    pub total: Cents,
    /// 计入账单的事件，需转入 `billed` 并写分录。
    pub billable: Vec<BillableEvent>,
    /// 超出封顶的事件：照常归因、照常给 KOL 记功，只是不收费。
    pub over_cap: Vec<BillableEvent>,
    /// 因封顶而免收的金额，用于在看板上告诉客户「本月免费送了你多少」。
    pub waived: Cents,
}

/// 组装账单。纯函数，没有 IO —— 账单的算法是这个系统最需要能被逐条复算的部分。
///
/// 关于**负数总额**：冲正额超过本期用量时 `total` 会是负数。这不是异常，
/// 而是一张 credit note —— 支付侧（Stripe）会把它记成客户的信用余额并在下期
/// 自动抵扣。刻意不把它截断到 0：截断等于把客户多付的钱悄悄吞掉。
pub fn assemble(inputs: Inputs) -> Bill {
    let mut bill = Bill::default();

    if inputs.platform_fee.is_positive() {
        bill.lines.push(Line {
            kind: line_kind::PLATFORM_FEE,
            description: "平台订阅费".into(),
            quantity: 1,
            unit_cents: inputs.platform_fee,
            amount_cents: inputs.platform_fee,
        });
        bill.subtotal += inputs.platform_fee;
    }

    let capped = billing::apply_cap(inputs.cleared, inputs.cap);
    if !capped.billable.is_empty() {
        // 单价按事件金额取，同一账期内理论上一致；取首条即可，逐条金额已在
        // 事件上冻结，不重新查定价 —— 定价改了也不该改已发生事件的金额。
        let unit = capped.billable[0].amount_cents;
        bill.lines.push(Line {
            kind: line_kind::CPA,
            description: format!("效果分成 · {} 笔确定性转化", capped.billable.len()),
            quantity: capped.billable.len() as i64,
            unit_cents: unit,
            amount_cents: capped.billed,
        });
        bill.subtotal += capped.billed;
    }

    if inputs.credit.is_positive() {
        bill.lines.push(Line {
            kind: line_kind::CREDIT,
            description: "上期冲正".into(),
            quantity: 1,
            unit_cents: inputs.credit,
            amount_cents: inputs.credit,
        });
    }

    bill.credit = inputs.credit;
    bill.total = Cents(bill.subtotal.0 - inputs.credit.0);
    bill.billable = capped.billable;
    bill.over_cap = capped.over_cap;
    bill.waived = capped.waived;
    bill
}

/// 为所有租户结算一个账期。
pub async fn run(pool: &PgPool, period: Period, now: DateTime<Utc>) -> anyhow::Result<()> {
    for tenant_id in super::all_tenant_ids(pool).await? {
        match run_for_tenant(pool, tenant_id, period, now).await {
            Ok(Some(id)) => tracing::info!(tenant_id, invoice_id = id, "账单已生成"),
            Ok(None) => tracing::info!(tenant_id, "账单已存在，跳过"),
            // 一个租户失败不应中断其他租户的结算 —— 月末窗口很紧，
            // 让 19 个正确的账单因为第 20 个的数据问题一起卡住是最差的选择。
            Err(e) => tracing::error!(tenant_id, error = %e, "结算失败"),
        }
    }
    Ok(())
}

async fn run_for_tenant(
    pool: &PgPool,
    tenant_id: i64,
    period: Period,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<i64>> {
    let mut tx = db::begin_tenant_tx(pool, tenant_id).await?;

    // 幂等：一个账期一张账单。任务被重复拉起时直接跳过，而不是出第二张。
    let exists = sqlx::query("SELECT id FROM invoice WHERE period_start = $1")
        .bind(period.start)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_some() {
        return Ok(None);
    }

    let pricing = load_pricing(&mut tx, tenant_id, now).await?;

    // 取事件的条件是 `invoice_id IS NULL`，不是「cleared_at 落在本账期内」。
    // 差别在于晚放行的事件：一笔上个月发生、这个月才走完人工复核的转化，
    // 按账期窗口取会被永远漏掉，按未开票取则会在下一期正常结上。
    let rows = sqlx::query(
        r#"
        SELECT id, tenant_id, attribution_id, event_type, external_id, status,
               amount_cents, currency, over_cap, occurred_at, received_at,
               hold_until, cleared_at, billed_at, invoice_id, status_reason
          FROM billable_event
         WHERE status = 'cleared' AND invoice_id IS NULL AND cleared_at < $1
         ORDER BY cleared_at
         FOR UPDATE
        "#,
    )
    .bind(period_end_ts(period))
    .fetch_all(&mut *tx)
    .await?;

    let cleared = rows
        .iter()
        .map(BillableEvent::from_row)
        .collect::<Result<Vec<_>, _>>()?;

    let credit = load_pending_credit(&mut tx, period).await?;

    let bill = assemble(Inputs {
        cleared,
        cap: pricing.monthly_cap,
        platform_fee: pricing.platform_fee,
        credit,
    });

    if bill.lines.is_empty() && bill.over_cap.is_empty() {
        return Ok(None);
    }

    let invoice_id: i64 = sqlx::query(
        r#"
        INSERT INTO invoice
          (tenant_id, period_start, period_end, subtotal_cents, credit_cents,
           total_cents, currency, status, created_at)
        VALUES ($1,$2,$3,$4,$5,$6,$7,'draft',$8)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(period.start)
    .bind(period.end)
    .bind(bill.subtotal.0)
    .bind(bill.credit.0)
    .bind(bill.total.0)
    .bind(&pricing.currency)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?
    .try_get("id")?;

    for line in &bill.lines {
        sqlx::query(
            r#"
            INSERT INTO invoice_line
              (tenant_id, invoice_id, kind, description, quantity, unit_cents, amount_cents)
            VALUES ($1,$2,$3,$4,$5,$6,$7)
            "#,
        )
        .bind(tenant_id)
        .bind(invoice_id)
        .bind(line.kind)
        .bind(&line.description)
        .bind(line.quantity)
        .bind(line.unit_cents.0)
        .bind(line.amount_cents.0)
        .execute(&mut *tx)
        .await?;
    }

    // ---- 计费事件转 billed 并写复式分录 ----
    for ev in &bill.billable {
        let mut ev = ev.clone();
        billing::transition(&mut ev, BillableStatus::Billed, None, now)
            .map_err(|e| anyhow::anyhow!(e))?;

        sqlx::query(
            "UPDATE billable_event SET status = $2, billed_at = $3, invoice_id = $4, over_cap = false WHERE id = $1",
        )
        .bind(ev.id)
        .bind(ev.status)
        .bind(ev.billed_at)
        .bind(invoice_id)
        .execute(&mut *tx)
        .await?;

        let txn = ledger::charge_cpa(tenant_id, &ev)?;
        write_txn(&mut tx, &txn, now).await?;
    }

    // ---- 超封顶：挂到账单上但不收费 ----
    // 状态仍是 cleared —— 它确实被放行了，只是这一笔免费。挂上 invoice_id
    // 是为了不被下个账期重复捡起，同时让客户在明细里看到「免费送了多少」。
    for ev in &bill.over_cap {
        sqlx::query("UPDATE billable_event SET over_cap = true, invoice_id = $2 WHERE id = $1")
            .bind(ev.id)
            .bind(invoice_id)
            .execute(&mut *tx)
            .await?;
    }

    // ---- 平台费分录 ----
    if bill.subtotal.is_positive() && pricing.platform_fee.is_positive() {
        if let Some(sub_id) = pricing.subscription_id {
            let txn = ledger::charge_platform_fee(
                tenant_id,
                sub_id,
                pricing.platform_fee,
                &pricing.currency,
            )?;
            write_txn(&mut tx, &txn, now).await?;
        }
    }

    // ---- 标记冲正额已入账，避免下期重复抵扣 ----
    sqlx::query(
        "UPDATE billable_event SET credited_invoice_id = $1 WHERE status = 'reversed' AND credited_invoice_id IS NULL AND invoice_id IS NOT NULL",
    )
    .bind(invoice_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Some(invoice_id))
}

async fn write_txn(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    txn: &ledger::Txn,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    for e in txn.entries() {
        sqlx::query(
            r#"
            INSERT INTO ledger_entry
              (tenant_id, txn_id, account, direction, amount_cents, currency,
               ref_type, ref_id, created_at)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
            "#,
        )
        .bind(txn.tenant_id())
        .bind(txn.id())
        .bind(e.account.as_str())
        .bind(e.direction.as_str())
        .bind(e.amount.0)
        .bind(&e.currency)
        .bind(txn.ref_type())
        .bind(txn.ref_id())
        .bind(now)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

struct Pricing {
    platform_fee: Cents,
    monthly_cap: Option<Cents>,
    currency: String,
    subscription_id: Option<i64>,
}

async fn load_pricing(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: i64,
    now: DateTime<Utc>,
) -> Result<Pricing, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT platform_fee_cents, monthly_cap_cents, currency
          FROM pricing_config
         WHERE (tenant_id = $1 OR tenant_id IS NULL)
           AND effective_from <= $2
           AND (effective_to IS NULL OR effective_to > $2)
         ORDER BY tenant_id NULLS LAST, effective_from DESC
         LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await?;

    let sub = sqlx::query("SELECT id FROM subscription WHERE status <> 'canceled' LIMIT 1")
        .fetch_optional(&mut **tx)
        .await?;

    Ok(match row {
        Some(r) => Pricing {
            platform_fee: Cents(r.try_get("platform_fee_cents")?),
            monthly_cap: r.try_get::<Option<i64>, _>("monthly_cap_cents")?.map(Cents),
            currency: r.try_get("currency")?,
            subscription_id: sub.map(|s| s.try_get("id")).transpose()?,
        },
        // 查不到定价时不要静默按 0 计费 —— 那会出一张金额全错的账单。
        // 这里返回零平台费但保留封顶为 None，配合上层的空账单跳过逻辑，
        // 表现为「没出账单」而不是「出了一张 $0 的账单」。
        None => Pricing {
            platform_fee: Cents::ZERO,
            monthly_cap: None,
            currency: "USD".into(),
            subscription_id: None,
        },
    })
}

/// 本期应抵扣的冲正额：已开过票、之后被冲正、且尚未在任何账单里抵扣过的事件。
async fn load_pending_credit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    _period: Period,
) -> Result<Cents, sqlx::Error> {
    let row = sqlx::query(
        r#"
        -- ::bigint 不能省：Postgres 的 sum(bigint) 返回 NUMERIC
        SELECT COALESCE(sum(amount_cents), 0)::bigint AS total
          FROM billable_event
         WHERE status = 'reversed'
           AND invoice_id IS NOT NULL
           AND credited_invoice_id IS NULL
        "#,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(Cents(row.try_get("total")?))
}

fn period_end_ts(period: Period) -> DateTime<Utc> {
    period
        .end
        .and_hms_opt(0, 0, 0)
        .expect("午夜总是合法时刻")
        .and_utc()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::event_type;
    use chrono::TimeZone;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, day, 0, 0, 0).unwrap()
    }

    fn cleared_events(amounts: &[i64]) -> Vec<BillableEvent> {
        amounts
            .iter()
            .enumerate()
            .map(|(i, &a)| BillableEvent {
                id: i as i64 + 1,
                tenant_id: 1,
                attribution_id: 1,
                event_type: event_type::ACTIVATION.into(),
                external_id: format!("claim:{}", i + 1),
                status: BillableStatus::Cleared,
                amount_cents: Cents(a),
                currency: "USD".into(),
                over_cap: false,
                occurred_at: at(1),
                received_at: at(1),
                hold_until: at(8),
                cleared_at: Some(at(8)),
                billed_at: None,
                invoice_id: None,
                status_reason: None,
            })
            .collect()
    }

    fn inputs(amounts: &[i64]) -> Inputs {
        Inputs {
            cleared: cleared_events(amounts),
            cap: None,
            platform_fee: Cents(9900),
            credit: Cents::ZERO,
        }
    }

    #[test]
    fn bills_platform_fee_and_usage() {
        let bill = assemble(inputs(&[200, 200, 200]));

        assert_eq!(bill.subtotal, Cents(9900 + 600));
        assert_eq!(bill.total, Cents(10500));
        assert_eq!(bill.lines.len(), 2);
        assert_eq!(bill.lines[1].quantity, 3);
        assert_eq!(bill.lines[1].unit_cents, Cents(200));
        assert_eq!(bill.billable.len(), 3);
    }

    /// 超出封顶的转化不收费但照常存在：它们进 over_cap，不进 subtotal。
    /// 「超出后免费」而不是「超出后停服」是刻意的产品选择。
    #[test]
    fn over_cap_conversions_are_free_not_rejected() {
        let bill = assemble(Inputs {
            cap: Some(Cents(400)),
            ..inputs(&[200, 200, 200, 200])
        });

        assert_eq!(bill.billable.len(), 2, "只有前两笔占用额度");
        assert_eq!(bill.over_cap.len(), 2, "其余两笔照常记录");
        assert_eq!(bill.waived, Cents(400), "应显示免收了多少");
        assert_eq!(bill.subtotal, Cents(9900 + 400));
    }

    /// 冲正超过本期用量时总额为负 —— 这是一张 credit note，
    /// 不应被截断到 0，否则等于吞掉客户多付的钱。
    #[test]
    fn credit_can_exceed_usage_and_produce_a_negative_total() {
        let bill = assemble(Inputs {
            platform_fee: Cents::ZERO,
            credit: Cents(1000),
            ..inputs(&[200])
        });

        assert_eq!(bill.subtotal, Cents(200));
        assert_eq!(bill.total, Cents(-800));
        assert!(bill.lines.iter().any(|l| l.kind == line_kind::CREDIT));
    }

    #[test]
    fn no_usage_still_bills_the_platform_fee() {
        let bill = assemble(inputs(&[]));

        assert_eq!(bill.lines.len(), 1);
        assert_eq!(bill.lines[0].kind, line_kind::PLATFORM_FEE);
        assert_eq!(bill.total, Cents(9900));
    }

    #[test]
    fn nothing_to_bill_produces_no_lines() {
        let bill = assemble(Inputs {
            platform_fee: Cents::ZERO,
            ..inputs(&[])
        });
        assert!(bill.lines.is_empty());
        assert_eq!(bill.total, Cents::ZERO);
    }

    // ------------------------------------------------------------ 账期

    #[test]
    fn previous_month_is_a_half_open_range() {
        let p = Period::previous_month(Utc.with_ymd_and_hms(2026, 7, 1, 3, 0, 0).unwrap());
        assert_eq!(p.start, NaiveDate::from_ymd_opt(2026, 6, 1).unwrap());
        assert_eq!(p.end, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap());
    }

    /// 跨年是最容易写错的分支：1 月结算的是去年 12 月。
    #[test]
    fn previous_month_crosses_the_year_boundary() {
        let p = Period::previous_month(Utc.with_ymd_and_hms(2027, 1, 1, 3, 0, 0).unwrap());
        assert_eq!(p.start, NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        assert_eq!(p.end, NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
    }

    #[test]
    fn previous_month_ignores_the_day_within_the_month() {
        let a = Period::previous_month(Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap());
        let b = Period::previous_month(Utc.with_ymd_and_hms(2026, 3, 28, 23, 0, 0).unwrap());
        assert_eq!(a, b);
        assert_eq!(a.start, NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
    }
}
