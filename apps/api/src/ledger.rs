//! Double-entry ledger.
//!
//! Constraint C3: the ledger is append-only. Refunds, chargebacks, and post-hoc fraud findings
//! are expressed by writing reversing entries, not UPDATEing originals. Any point-in-time balance
//! can be replayed from entries — full, unmodified evidence for customer appeals.

use std::collections::HashMap;

use uuid::Uuid;

use crate::models::{Account, BillableEvent, Cents, Direction};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LedgerError {
    #[error("ledger: debits and credits unbalanced debit={debit} credit={credit}")]
    Unbalanced { debit: Cents, credit: Cents },
    #[error("ledger: transaction has no entries")]
    Empty,
    #[error("ledger: single transaction mixes currencies {a} / {b}")]
    MixedCurrency { a: String, b: String },
    #[error("ledger: entry amount must be positive, account {account} amount {amount}")]
    NonPositive { account: String, amount: Cents },
}

/// One ledger entry. Amount is always positive; direction carries sign.
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

/// One transaction: a set of balanced debit/credit entries.
///
/// Fields are private and construction goes only through [`Txn::try_new`] — **an unbalanced
/// `Txn` cannot exist at the type level**. Second direct benefit of Rust: core ledger invariants
/// enforced in the constructor, not by callers remembering to call `validate()`.
///
/// Once unbalanced entries hit the database, every reconciliation breaks, and because the ledger
/// is append-only, repair means more compensating entries. Reject at construction instead.
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

    /// Build a reversal transaction with directions flipped from the original.
    ///
    /// Deliberately does not mutate or delete original entries. Original and reversal coexist on
    /// the ledger with net zero — that traceability is auditability.
    ///
    /// Returns `Txn` not `Result`: a successfully constructed original is balanced; flipping every
    /// direction preserves balance.
    // TODO(appeal): appeal and refund endpoints will call this — remove allow when shipped.
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

/// One CPA charge:
///
/// ```text
/// D  tenant_receivable   customer owes us
/// C  platform_revenue    recognise revenue
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

/// One platform subscription fee.
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

/// Net balance by account for a slice of entries (debit positive, credit negative) — used in
/// invariant checks and reconciliation.
///
/// `ledger-audit` does not use this: it aggregates in SQL to avoid loading all entries into memory.
/// This function is for ad-hoc reconciliation and per-transaction checks.
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
        let txn = charge_cpa(1, &event(200)).expect("should construct");
        let bal = balance(txn.entries());

        assert_eq!(
            bal[&Account::TenantReceivable],
            200,
            "customer receivable increases"
        );
        assert_eq!(
            bal[&Account::PlatformRevenue],
            -200,
            "revenue credit increases"
        );
    }

    /// After reversal, net must be zero and original entries must remain — that is auditability.
    #[test]
    fn reverse_nets_to_zero_and_keeps_original() {
        let orig = charge_cpa(1, &event(200)).unwrap();
        let rev = orig.reverse("reversal", 42);

        assert_ne!(
            orig.id(),
            rev.id(),
            "reversal is a new transaction, not an in-place edit"
        );

        let combined: Vec<Entry> = orig
            .entries()
            .iter()
            .chain(rev.entries())
            .cloned()
            .collect();
        for (account, amount) in balance(&combined) {
            assert_eq!(
                amount, 0,
                "account {account:?} should net to zero after reversal"
            );
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

    /// Amounts must be positive; direction carries sign. Negative amounts allow two encodings of
    /// the same economic event — reconciliation cannot tell which is canonical.
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
