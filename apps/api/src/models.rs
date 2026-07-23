//! Domain types and cross-module shared enums.
//!
//! Carries the design doc §3 domain model plus two non-negotiable rules:
//!
//! - Mapping from attribution method to billability (constraint C1)
//! - Billable-event state machine (constraint C3)
//!
//! Both use exhaustive `match`, not lookup tables. That is a direct benefit of Rust: adding an
//! attribution method forces an explicit `is_billable` decision — you cannot forget to register it
//! and silently fall through. Billing policy should not depend on humans maintaining a table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- amounts

/// Monetary amount in the smallest currency unit (cent).
///
/// Newtype rather than bare `i64`: the system has many i64s (counts, IDs, durations); mixing them
/// with money is the easiest and hardest-to-spot class of bug.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize, sqlx::Type,
)]
#[sqlx(transparent)]
pub struct Cents(pub i64);

impl Cents {
    pub const ZERO: Cents = Cents(0);

    pub fn is_positive(self) -> bool {
        self.0 > 0
    }
}

impl std::ops::Add for Cents {
    type Output = Cents;
    fn add(self, rhs: Cents) -> Cents {
        Cents(self.0 + rhs.0)
    }
}

impl std::ops::AddAssign for Cents {
    fn add_assign(&mut self, rhs: Cents) {
        self.0 += rhs.0;
    }
}

impl std::fmt::Display for Cents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{:02}", self.0 / 100, (self.0 % 100).abs())
    }
}

// ---------------------------------------------------------------- attribution

/// Attribution method. Values match the database `attribution_method` enum one-to-one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "attribution_method", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AttributionMethod {
    /// Claim-code redemption — the only billable path on iOS
    DeterministicCode,
    /// Android Play Install Referrer — zero user action
    InstallReferrer,
    /// Direct launch for installed users; parameters in the URL
    UniversalLink,
    /// Clipboard match — may improve dashboard conversion, not billing
    ClipboardMatch,
    /// Fingerprint / time-window match — collapsed accuracy on iOS 17+, analytics only
    Probabilistic,
}

impl AttributionMethod {
    /// Whether this attribution method is billable.
    ///
    /// **Foundation of the entire commercial model.** Mid-tier performance share bills only
    /// deterministic attribution; probabilistic conversions may enter dashboards, never invoices
    /// (constraint C1).
    ///
    /// Context: after iOS 17+, reliable user-level deferred deep linking is gone — no IDFA,
    /// fingerprint accuracy collapsed, AdAttributionKit returns aggregated delayed signals only.
    /// Billing on probabilistic match means charging customers for revenue we cannot verify.
    pub fn is_billable(self) -> bool {
        use AttributionMethod::*;
        match self {
            DeterministicCode | InstallReferrer | UniversalLink => true,
            ClipboardMatch | Probabilistic => false,
        }
    }

    /// Confidence score for dashboard tiering.
    pub fn confidence(self) -> i16 {
        use AttributionMethod::*;
        match self {
            DeterministicCode | InstallReferrer | UniversalLink => 100,
            ClipboardMatch => 60,
            Probabilistic => 30,
        }
    }
}

/// Attribution record — the system's trust anchor.
///
/// `evidence` stores the full input snapshot at decision time — the sole evidence source for KOL
/// appeals; append-only. `policy_version` points at a customer-visible rules document; changes
/// require a new version number.
// TODO(query): attribution query endpoint (GET /v1/attribution/:app_user_id) and appeal
// recomputation will read this — remove allow when shipped.
#[allow(dead_code)]
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Attribution {
    pub id: i64,
    pub tenant_id: i64,
    pub player_id: i64,
    pub kol_id: i64,
    pub campaign_id: i64,
    pub link_id: i64,
    pub method: AttributionMethod,
    pub confidence: i16,
    /// Redundant (derivable from `method`) but must be stored: billing rules change,
    /// yet invoiced decisions must be frozen.
    pub is_billable: bool,
    pub policy_version: String,
    pub touch_at: DateTime<Utc>,
    pub attributed_at: DateTime<Utc>,
    pub locked_until: DateTime<Utc>,
    pub evidence: serde_json::Value,
}

// ---------------------------------------------------------------- billing

/// Billable-event status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "billable_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum BillableStatus {
    /// Received; within hold period
    Pending,
    /// Risk hold; awaiting manual review
    Held,
    /// Cleared; eligible for invoicing
    Cleared,
    /// Invoiced
    Billed,
    /// Reversed (refund / post-hoc fraud determination)
    Reversed,
    /// Rejected; not billable
    Rejected,
}

impl BillableStatus {
    /// Whether transition from current status to `to` is allowed.
    ///
    /// ```text
    ///               ┌──────────► Rejected
    ///               │
    ///  Pending ──► Cleared ──► Billed
    ///     │          ▲            │
    ///     ▼          │            ▼
    ///   Held ────────┘         Reversed
    /// ```
    ///
    /// `Billed` may still transition to `Reversed` — reversal posts as credit in the next period,
    /// without retroactively editing issued invoices.
    pub fn can_transition_to(self, to: BillableStatus) -> bool {
        use BillableStatus::*;
        matches!(
            (self, to),
            (Pending, Cleared)
                | (Pending, Held)
                | (Pending, Rejected)
                | (Held, Cleared)
                | (Held, Rejected)
                | (Cleared, Billed)
                | (Cleared, Reversed)
                | (Cleared, Rejected)
                | (Billed, Reversed)
        )
    }
}

/// Illegal state transition.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("billable event: illegal state transition {from:?} -> {to:?}")]
pub struct IllegalTransition {
    pub from: BillableStatus,
    pub to: BillableStatus,
}

/// Event type identifiers.
pub mod event_type {
    pub const ACTIVATION: &str = "activation";
    pub const IAP_PURCHASE: &str = "iap_purchase";
}

/// Billable event — atomic unit of revenue.
///
/// Only conversions with `Attribution::is_billable == true` produce `BillableEvent`; non-billable
/// conversions go to analytics (ClickHouse) only, not this table.
// Fields mirror the table; `FromRow` reads the full row. external_id, occurred_at, etc. are
// consumed by detail export and variance views — not implemented yet.
#[allow(dead_code)]
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BillableEvent {
    pub id: i64,
    pub tenant_id: i64,
    pub attribution_id: i64,
    pub event_type: String,
    /// Main-app-side unique ID; idempotency key
    pub external_id: String,
    pub status: BillableStatus,
    pub amount_cents: Cents,
    pub currency: String,
    pub over_cap: bool,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub hold_until: DateTime<Utc>,
    pub cleared_at: Option<DateTime<Utc>>,
    pub billed_at: Option<DateTime<Utc>>,
    pub invoice_id: Option<i64>,
    pub status_reason: Option<String>,
}

// ---------------------------------------------------------------- ledger

/// Ledger account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Account {
    /// Amount owed to us by the customer
    TenantReceivable,
    /// Platform revenue
    PlatformRevenue,
    /// Amount we owe KOLs
    KolPayable,
    /// Reversal clearing
    ReversalClearing,
}

impl Account {
    pub fn as_str(self) -> &'static str {
        match self {
            Account::TenantReceivable => "tenant_receivable",
            Account::PlatformRevenue => "platform_revenue",
            Account::KolPayable => "kol_payable",
            Account::ReversalClearing => "reversal_clearing",
        }
    }
}

/// Debit/credit direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Debit,
    Credit,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Debit => "D",
            Direction::Credit => "C",
        }
    }

    /// Pairs with `Txn::reverse`; no online callers until reversal path ships.
    #[allow(dead_code)]
    pub fn flip(self) -> Direction {
        match self {
            Direction::Debit => Direction::Credit,
            Direction::Credit => Direction::Debit,
        }
    }
}

// ---------------------------------------------------------------- other

/// Claim-code status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "claim_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    Issued,
    Redeemed,
    Expired,
    Voided,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the commercial-model foundation. If Probabilistic were accidentally billable,
    /// invoices would immediately lose credibility.
    #[test]
    fn only_deterministic_methods_are_billable() {
        use AttributionMethod::*;
        for m in [DeterministicCode, InstallReferrer, UniversalLink] {
            assert!(m.is_billable(), "{m:?} should be billable");
            assert_eq!(m.confidence(), 100, "{m:?} confidence should be 100");
        }
        for m in [ClipboardMatch, Probabilistic] {
            assert!(!m.is_billable(), "{m:?} should not be billable");
            assert!(m.confidence() < 100, "{m:?} confidence should be below 100");
        }
    }

    #[test]
    fn billable_state_machine_allows_legal_transitions() {
        use BillableStatus::*;
        let legal = [
            (Pending, Cleared),
            (Pending, Held),
            (Pending, Rejected),
            (Held, Cleared),
            (Held, Rejected),
            (Cleared, Billed),
            (Cleared, Reversed),
            // Invoiced events may still be reversed — credit in the next period
            (Billed, Reversed),
        ];
        for (from, to) in legal {
            assert!(
                from.can_transition_to(to),
                "{from:?} -> {to:?} should be legal"
            );
        }
    }

    #[test]
    fn billable_state_machine_rejects_illegal_transitions() {
        use BillableStatus::*;
        let illegal = [
            (Billed, Cleared),   // cannot roll back to cleared
            (Reversed, Cleared), // reversal is terminal
            (Rejected, Cleared), // rejection is terminal
            (Pending, Billed),   // must pass hold first
            (Held, Billed),      // held cannot invoice directly
        ];
        for (from, to) in illegal {
            assert!(
                !from.can_transition_to(to),
                "{from:?} -> {to:?} should be illegal"
            );
        }
    }

    #[test]
    fn cents_displays_as_decimal() {
        assert_eq!(Cents(200).to_string(), "2.00");
        assert_eq!(Cents(9900).to_string(), "99.00");
        assert_eq!(Cents(5).to_string(), "0.05");
    }
}
