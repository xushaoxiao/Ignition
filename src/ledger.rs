//! 复式记账。
//!
//! 约束 C3：账本只可追加，不可修改。退款、拒付、事后判定作弊都通过写一笔
//! 反向分录来表达，而不是 UPDATE 原记录。这样任何一个时点的余额都可以由分录
//! 重放得出，客户申诉时能拿出完整的、未被改动过的证据链。

use std::collections::HashMap;

use uuid::Uuid;

use crate::models::{Account, BillableEvent, Cents, Direction};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LedgerError {
    #[error("账本: 借贷不平 debit={debit} credit={credit}")]
    Unbalanced { debit: Cents, credit: Cents },
    #[error("账本: 交易不含任何分录")]
    Empty,
    #[error("账本: 单笔交易混用币种 {a} / {b}")]
    MixedCurrency { a: String, b: String },
    #[error("账本: 分录金额必须为正，科目 {account} 金额 {amount}")]
    NonPositive { account: String, amount: Cents },
}

/// 一条分录。金额恒为正，方向由 `direction` 表达。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub account: Account,
    pub direction: Direction,
    pub amount: Cents,
    pub currency: String,
}

impl Entry {
    pub fn new(account: Account, direction: Direction, amount: Cents, currency: &str) -> Self {
        Entry {
            account,
            direction,
            amount,
            currency: currency.to_string(),
        }
    }
}

/// 一笔交易，由若干条借贷平衡的分录组成。
///
/// 字段私有且只能经 [`Txn::try_new`] 构造 —— **一个不平衡的 `Txn` 在类型层面
/// 不可能存在**。这是选 Rust 的第二个直接收益：账本的核心不变量由构造函数
/// 强制，而不是靠调用方记得先调一次 `validate()`。
///
/// 一旦不平衡的分录写进库，后续所有对账都会失效，而由于账本不可修改，
/// 修复只能靠更多的补偿分录。所以宁可在构造时就拒绝。
#[derive(Debug, Clone)]
pub struct Txn {
    id: Uuid,
    tenant_id: i64,
    ref_type: String,
    ref_id: i64,
    entries: Vec<Entry>,
}

impl Txn {
    pub fn try_new(
        tenant_id: i64,
        ref_type: &str,
        ref_id: i64,
        entries: Vec<Entry>,
    ) -> Result<Self, LedgerError> {
        Self::validate(&entries)?;
        Ok(Txn {
            id: Uuid::new_v4(),
            tenant_id,
            ref_type: ref_type.to_string(),
            ref_id,
            entries,
        })
    }

    fn validate(entries: &[Entry]) -> Result<(), LedgerError> {
        let first = entries.first().ok_or(LedgerError::Empty)?;
        let currency = &first.currency;

        let (mut debit, mut credit) = (Cents::ZERO, Cents::ZERO);
        for e in entries {
            if !e.amount.is_positive() {
                return Err(LedgerError::NonPositive {
                    account: e.account.as_str().to_string(),
                    amount: e.amount,
                });
            }
            if &e.currency != currency {
                return Err(LedgerError::MixedCurrency {
                    a: currency.clone(),
                    b: e.currency.clone(),
                });
            }
            match e.direction {
                Direction::Debit => debit += e.amount,
                Direction::Credit => credit += e.amount,
            }
        }
        if debit != credit {
            return Err(LedgerError::Unbalanced { debit, credit });
        }
        Ok(())
    }

    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn tenant_id(&self) -> i64 {
        self.tenant_id
    }
    pub fn ref_type(&self) -> &str {
        &self.ref_type
    }
    pub fn ref_id(&self) -> i64 {
        self.ref_id
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// 构造一笔冲正交易：方向与原交易完全相反。
    ///
    /// 刻意不去修改原分录，也不去「删掉」它。原交易与冲正交易在账本上并存，
    /// 净额为零，但两者都留有痕迹 —— 这正是可审计性的来源。
    ///
    /// 返回 `Txn` 而非 `Result`：原交易既然构造成功就一定是平衡的，
    /// 逐条翻转方向后必然仍然平衡。
    // TODO(appeal): 申诉与退款接口落地后会调用它，届时移除 allow。
    #[allow(dead_code)]
    pub fn reverse(&self, ref_type: &str, ref_id: i64) -> Txn {
        let entries = self
            .entries
            .iter()
            .map(|e| Entry {
                account: e.account,
                direction: e.direction.flip(),
                amount: e.amount,
                currency: e.currency.clone(),
            })
            .collect();
        Txn {
            id: Uuid::new_v4(),
            tenant_id: self.tenant_id,
            ref_type: ref_type.to_string(),
            ref_id,
            entries,
        }
    }
}

/// 一笔 CPA 计费：
///
/// ```text
/// D  tenant_receivable   客户应付我方
/// C  platform_revenue    确认收入
/// ```
pub fn charge_cpa(tenant_id: i64, ev: &BillableEvent) -> Result<Txn, LedgerError> {
    Txn::try_new(
        tenant_id,
        "billable_event",
        ev.id,
        vec![
            Entry::new(
                Account::TenantReceivable,
                Direction::Debit,
                ev.amount_cents,
                &ev.currency,
            ),
            Entry::new(
                Account::PlatformRevenue,
                Direction::Credit,
                ev.amount_cents,
                &ev.currency,
            ),
        ],
    )
}

/// 一笔平台订阅费。
pub fn charge_platform_fee(
    tenant_id: i64,
    subscription_id: i64,
    amount: Cents,
    currency: &str,
) -> Result<Txn, LedgerError> {
    Txn::try_new(
        tenant_id,
        "subscription",
        subscription_id,
        vec![
            Entry::new(
                Account::TenantReceivable,
                Direction::Debit,
                amount,
                currency,
            ),
            Entry::new(
                Account::PlatformRevenue,
                Direction::Credit,
                amount,
                currency,
            ),
        ],
    )
}

/// 按科目汇总一组分录的净额（借为正，贷为负），用于不变量校验与对账。
///
/// `ledger-audit` 任务没有用它：那边直接在 SQL 里聚合，避免把全量分录读进
/// 内存。这个函数留给人工对账和单笔交易的即时校验。
#[allow(dead_code)]
pub fn balance(entries: &[Entry]) -> HashMap<Account, i64> {
    let mut out: HashMap<Account, i64> = HashMap::new();
    for e in entries {
        let delta = match e.direction {
            Direction::Debit => e.amount.0,
            Direction::Credit => -e.amount.0,
        };
        *out.entry(e.account).or_insert(0) += delta;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::BillableStatus;
    use chrono::Utc;

    fn event(amount: i64) -> BillableEvent {
        let now = Utc::now();
        BillableEvent {
            id: 42,
            tenant_id: 1,
            attribution_id: 1,
            event_type: crate::models::event_type::ACTIVATION.into(),
            external_id: "claim:1".into(),
            status: BillableStatus::Cleared,
            amount_cents: Cents(amount),
            currency: "USD".into(),
            over_cap: false,
            occurred_at: now,
            received_at: now,
            hold_until: now,
            cleared_at: None,
            billed_at: None,
            invoice_id: None,
            status_reason: None,
        }
    }

    #[test]
    fn charge_cpa_balances() {
        let txn = charge_cpa(1, &event(200)).expect("应构造成功");
        let bal = balance(txn.entries());

        assert_eq!(bal[&Account::TenantReceivable], 200, "客户应付增加");
        assert_eq!(bal[&Account::PlatformRevenue], -200, "收入贷方增加");
    }

    /// 冲正后净额必须归零，且原分录仍然存在 —— 这是可审计性的来源。
    #[test]
    fn reverse_nets_to_zero_and_keeps_original() {
        let orig = charge_cpa(1, &event(200)).unwrap();
        let rev = orig.reverse("reversal", 42);

        assert_ne!(orig.id(), rev.id(), "冲正是一笔新交易，不是修改原交易");

        let combined: Vec<Entry> = orig
            .entries()
            .iter()
            .chain(rev.entries())
            .cloned()
            .collect();
        for (account, amount) in balance(&combined) {
            assert_eq!(amount, 0, "科目 {account:?} 冲正后应归零");
        }
    }

    #[test]
    fn rejects_unbalanced() {
        let err = Txn::try_new(
            1,
            "test",
            1,
            vec![
                Entry::new(
                    Account::TenantReceivable,
                    Direction::Debit,
                    Cents(200),
                    "USD",
                ),
                Entry::new(
                    Account::PlatformRevenue,
                    Direction::Credit,
                    Cents(100),
                    "USD",
                ),
            ],
        )
        .unwrap_err();

        assert_eq!(
            err,
            LedgerError::Unbalanced {
                debit: Cents(200),
                credit: Cents(100)
            }
        );
    }

    #[test]
    fn rejects_mixed_currency() {
        let err = Txn::try_new(
            1,
            "test",
            1,
            vec![
                Entry::new(
                    Account::TenantReceivable,
                    Direction::Debit,
                    Cents(200),
                    "USD",
                ),
                Entry::new(
                    Account::PlatformRevenue,
                    Direction::Credit,
                    Cents(200),
                    "EUR",
                ),
            ],
        )
        .unwrap_err();

        assert!(matches!(err, LedgerError::MixedCurrency { .. }));
    }

    /// 金额必须为正，方向由 Direction 表达。允许负数会让同一笔账有两种写法，
    /// 对账时无法判断哪种是对的。
    #[test]
    fn rejects_non_positive() {
        let err = Txn::try_new(
            1,
            "test",
            1,
            vec![
                Entry::new(
                    Account::TenantReceivable,
                    Direction::Debit,
                    Cents(-200),
                    "USD",
                ),
                Entry::new(
                    Account::PlatformRevenue,
                    Direction::Credit,
                    Cents(-200),
                    "USD",
                ),
            ],
        )
        .unwrap_err();

        assert!(matches!(err, LedgerError::NonPositive { .. }));
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(
            Txn::try_new(1, "test", 1, vec![]).unwrap_err(),
            LedgerError::Empty
        );
    }
}
