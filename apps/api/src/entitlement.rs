//! Entitlement gating (design constraint C4).
//!
//! **No `if plan == "pro"`.** Not pedantry: early sales will promise "free Discord for now" or
//! "two extra groups". Hard-coded tier checks cannot honour that — you either invent fake plans or
//! sprinkle exceptions in code, and three months later nobody can say what a customer actually bought.
//!
//! Capabilities come from data: `plan_entitlement` for plan defaults,
//! `tenant_entitlement_override` for sales exceptions; overrides win and may carry expiry.

// Entitlement resolution is complete and tested, but concrete gate points (raw export, Discord
// channel, cohort analytics) are not wired yet — no callers. Remove when the first paid gate ships.
#![allow(dead_code)]

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

/// Entitlement keys. Central definitions avoid the same capability spelled two ways in code —
/// that failure mode is "feature silently off", painful to debug.
pub mod key {
    /// Channel count limit
    pub const CHANNEL_COUNT: &str = "channel.count";
    /// Discord extension
    pub const CHANNEL_DISCORD: &str = "channel.discord";
    /// WhatsApp extension
    pub const CHANNEL_WHATSAPP: &str = "channel.whatsapp";
    /// Custom templates
    pub const TEMPLATE_CUSTOM: &str = "template.custom";
    /// Three-metric dashboard
    pub const ANALYTICS_BASIC: &str = "analytics.basic";
    /// Cohort / retention analytics
    pub const ANALYTICS_COHORT: &str = "analytics.cohort";
    /// White-label branding
    pub const BRANDING_WHITELABEL: &str = "branding.whitelabel";
    /// KOL marketplace
    pub const MARKETPLACE_KOL: &str = "marketplace.kol";
    /// Performance-based billing enabled
    pub const BILLING_PERFORMANCE: &str = "billing.performance";
    /// Raw detail export
    pub const EXPORT_RAW: &str = "export.raw";
}

/// All entitlements currently effective for a tenant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entitlements {
    values: HashMap<String, Value>,
}

/// One entitlement grant from plan default or tenant override.
#[derive(Debug, Clone)]
pub struct Grant {
    pub key: String,
    pub value: Value,
    /// Only overrides have expiry; plan defaults are always `None`.
    pub expires_at: Option<DateTime<Utc>>,
}

impl Entitlements {
    /// Merge plan defaults and tenant overrides into the effective entitlement set.
    ///
    /// Overrides apply last and replace same-key plan defaults. Expired overrides are dropped,
    /// **not** treated as "off" — after drop, plan defaults return. That is how "free for a month"
    /// should behave when the month ends.
    pub fn resolve(plan: Vec<Grant>, overrides: Vec<Grant>, now: DateTime<Utc>) -> Entitlements {
        let mut values = HashMap::new();
        for g in plan {
            values.insert(g.key, g.value);
        }
        for g in overrides {
            if g.expires_at.is_some_and(|t| now >= t) {
                continue;
            }
            values.insert(g.key, g.value);
        }
        Entitlements { values }
    }

    /// Whether a boolean entitlement is enabled.
    ///
    /// **Default deny**: missing keys mean disabled. When adding a paid capability, forgetting to
    /// grant it on any plan means "nobody can use it" not "everyone gets it free" — the former gets
    /// reported, the latter does not.
    pub fn check(&self, key: &str) -> bool {
        match self.values.get(key) {
            Some(Value::Bool(b)) => *b,
            // `{"limit": n}` shape — present means enabled (n from limit())
            Some(Value::Object(o)) => o.contains_key("limit"),
            _ => false,
        }
    }

    /// Numeric cap for a limit-style entitlement. `None` means unset — callers should treat as deny.
    pub fn limit(&self, key: &str) -> Option<i64> {
        self.values.get(key)?.get("limit")?.as_i64()
    }

    /// Whether current usage is below the cap. Returns false when no cap is configured (default deny).
    pub fn within_limit(&self, key: &str, current: i64) -> bool {
        self.limit(key).is_some_and(|max| current < max)
    }
}

// ---------------------------------------------------------------- subscription service level

/// Service level implied by subscription status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceLevel {
    /// Full service
    Full,
    /// Read-only: dashboards and export work; links stop distributing; games not accepted
    ReadOnly,
}

/// Derive service level from subscription status.
///
/// **`past_due` stays full during grace.** Cutting service breaks KOL-side funnels and hurts the
/// customer's customers — churn beats payment recovery. After grace, degrade to read-only rather
/// than hard shutdown so data remains exportable and reactivation is possible.
pub fn service_level(
    status: SubscriptionStatus,
    grace_until: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> ServiceLevel {
    use SubscriptionStatus::*;
    match status {
        Trialing | Active => ServiceLevel::Full,
        // Missing grace is treated as expired — conservative beats indefinite free service from a missing field.
        PastDue => match grace_until {
            Some(t) if now < t => ServiceLevel::Full,
            _ => ServiceLevel::ReadOnly,
        },
        Paused | Canceled => ServiceLevel::ReadOnly,
    }
}

/// Subscription status. Values match database `subscription_status` enum one-to-one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "subscription_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    Trialing,
    Active,
    PastDue,
    Paused,
    Canceled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeDelta, TimeZone};
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 21, 12, 0, 0).unwrap()
    }

    fn grant(key: &str, value: Value) -> Grant {
        Grant {
            key: key.into(),
            value,
            expires_at: None,
        }
    }

    fn expiring(key: &str, value: Value, at: DateTime<Utc>) -> Grant {
        Grant {
            key: key.into(),
            value,
            expires_at: Some(at),
        }
    }

    /// Default deny: unconfigured capabilities are off. Wrong key spelling should mean "broken",
    /// not "free for everyone".
    #[test]
    fn unknown_keys_are_denied() {
        let e = Entitlements::default();
        assert!(!e.check(key::CHANNEL_DISCORD));
        assert!(!e.check("totally.made.up"));
        assert_eq!(e.limit(key::CHANNEL_COUNT), None);
        assert!(!e.within_limit(key::CHANNEL_COUNT, 0));
    }

    #[test]
    fn plan_defaults_apply() {
        let e = Entitlements::resolve(
            vec![
                grant(key::ANALYTICS_BASIC, json!(true)),
                grant(key::CHANNEL_COUNT, json!({"limit": 3})),
            ],
            vec![],
            now(),
        );
        assert!(e.check(key::ANALYTICS_BASIC));
        assert_eq!(e.limit(key::CHANNEL_COUNT), Some(3));
        assert!(e.within_limit(key::CHANNEL_COUNT, 2));
        assert!(!e.within_limit(key::CHANNEL_COUNT, 3));
    }

    /// Sales promise "free Discord for now" is one override row, not a code exception.
    #[test]
    fn override_beats_plan_default() {
        let e = Entitlements::resolve(
            vec![grant(key::CHANNEL_DISCORD, json!(false))],
            vec![grant(key::CHANNEL_DISCORD, json!(true))],
            now(),
        );
        assert!(e.check(key::CHANNEL_DISCORD));
    }

    /// Expired override falls back to plan default — not zero. "Extra month" ends at purchased tier.
    #[test]
    fn expired_override_falls_back_to_plan_default() {
        let plan = vec![grant(key::CHANNEL_COUNT, json!({"limit": 1}))];
        let promo = vec![expiring(
            key::CHANNEL_COUNT,
            json!({"limit": 10}),
            now() - TimeDelta::days(1),
        )];

        let e = Entitlements::resolve(plan, promo, now());
        assert_eq!(e.limit(key::CHANNEL_COUNT), Some(1), "should fall back to plan default");
    }

    #[test]
    fn unexpired_override_still_applies() {
        let e = Entitlements::resolve(
            vec![grant(key::CHANNEL_COUNT, json!({"limit": 1}))],
            vec![expiring(
                key::CHANNEL_COUNT,
                json!({"limit": 10}),
                now() + TimeDelta::days(1),
            )],
            now(),
        );
        assert_eq!(e.limit(key::CHANNEL_COUNT), Some(10));
    }

    #[test]
    fn explicit_false_disables() {
        let e = Entitlements::resolve(vec![grant(key::EXPORT_RAW, json!(false))], vec![], now());
        assert!(!e.check(key::EXPORT_RAW));
    }

    // ------------------------------------------------------------ service level

    /// past_due within grace stays full — outage hurts the customer's customers; churn beats payment.
    #[test]
    fn past_due_keeps_full_service_within_grace() {
        assert_eq!(
            service_level(
                SubscriptionStatus::PastDue,
                Some(now() + TimeDelta::days(3)),
                now()
            ),
            ServiceLevel::Full
        );
    }

    #[test]
    fn past_due_degrades_to_read_only_after_grace() {
        assert_eq!(
            service_level(
                SubscriptionStatus::PastDue,
                Some(now() - TimeDelta::hours(1)),
                now()
            ),
            ServiceLevel::ReadOnly
        );
    }

    /// Missing grace_until treated as expired — conservative is less service, not free forever.
    #[test]
    fn past_due_without_grace_is_read_only() {
        assert_eq!(
            service_level(SubscriptionStatus::PastDue, None, now()),
            ServiceLevel::ReadOnly
        );
    }

    #[test]
    fn trialing_gets_full_service() {
        assert_eq!(
            service_level(SubscriptionStatus::Trialing, None, now()),
            ServiceLevel::Full
        );
    }

    #[test]
    fn paused_and_canceled_are_read_only() {
        for s in [SubscriptionStatus::Paused, SubscriptionStatus::Canceled] {
            assert_eq!(
                service_level(s, None, now()),
                ServiceLevel::ReadOnly,
                "{s:?}"
            );
        }
    }
}
