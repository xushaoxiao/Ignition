//! Versioned attribution policy definitions.

use chrono::TimeDelta;

use crate::models::event_type;

/// One version of the attribution policy.
///
/// **This table is the product itself**: customers reconcile against it, KOLs appeal
/// against it. Policy changes must ship under a new version number with advance notice —
/// never silently (constraint C2). Every `Attribution` record stores the
/// `policy_version` in force at the time, so any historical invoice can be recomputed
/// exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub version: &'static str,
    /// Click-attribution window; clicks older than this are invalid.
    pub click_window: TimeDelta,
    /// Claim-code lifetime.
    pub claim_code_ttl: TimeDelta,
    /// Attribution lock period: conversions by this Player count for the original KOL.
    pub lock_period: TimeDelta,
    /// Hold period for activation events; reversible while pending.
    pub activation_hold: TimeDelta,
    /// Hold period for `iap_purchase`; must cover the App Store refund window.
    pub purchase_hold: TimeDelta,
}

/// Currently effective attribution policy. See `docs/product/attribution-policy-v1.md`.
pub const V1: Policy = Policy {
    version: "v1",
    click_window: TimeDelta::days(7),
    claim_code_ttl: TimeDelta::hours(24),
    lock_period: TimeDelta::days(90),
    // Activation is confirmed by our redemption path; 7 days covers the anomaly-discovery window.
    activation_hold: TimeDelta::days(7),
    // Future GMV share must cover the App Store refund window, hence 35 days.
    purchase_hold: TimeDelta::days(35),
};

#[derive(Debug, thiserror::Error)]
#[error("attribution: unknown attribution policy version {0:?}")]
pub struct UnknownPolicyVersion(pub String);

/// Resolve policy by version string, for appeal recomputation of historical records.
pub fn by_version(v: &str) -> Result<Policy, UnknownPolicyVersion> {
    match v {
        "v1" => Ok(V1),
        other => Err(UnknownPolicyVersion(other.to_string())),
    }
}

impl Policy {
    /// Hold period for a given event type.
    ///
    /// Unknown types fall back to 7 days — better to hold a little longer than bill immediately.
    pub fn hold_period(&self, event_type: &str) -> TimeDelta {
        match event_type {
            event_type::ACTIVATION => self.activation_hold,
            event_type::IAP_PURCHASE => self.purchase_hold,
            _ => TimeDelta::days(7),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_is_resolvable_by_version() {
        assert_eq!(by_version("v1").unwrap(), V1);
        assert!(by_version("v99").is_err());
    }

    /// IAP hold must be materially longer than activation — it must cover the App Store
    /// refund window, otherwise we pay the KOL and only then discover a refunded order.
    #[test]
    fn purchase_hold_covers_refund_window() {
        assert!(V1.purchase_hold > V1.activation_hold);
        assert!(V1.purchase_hold >= TimeDelta::days(30));
    }

    #[test]
    fn unknown_event_type_falls_back_to_conservative_hold() {
        assert_eq!(V1.hold_period("something_new"), TimeDelta::days(7));
        assert_eq!(V1.hold_period(event_type::ACTIVATION), TimeDelta::days(7));
        assert_eq!(
            V1.hold_period(event_type::IAP_PURCHASE),
            TimeDelta::days(35)
        );
    }
}
