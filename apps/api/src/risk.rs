//! Risk L1 hard constraints and L2 signal definitions.
//!
//! Risk serves two roles: protect reward cost and protect billing accuracy. The latter matters
//! more — a conversion judged fraudulent after we charged the customer damages trust, not just money.

use chrono::Duration;
use serde::{Deserialize, Serialize};

/// L1 rule action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Pass,
    /// Mark held: event is recorded but excluded from billing pending manual review
    Hold,
    Deny,
}

/// One risk decision.
#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub action: Action,
    pub rule: &'static str,
    pub detail: serde_json::Value,
}

impl Verdict {
    fn pass() -> Self {
        Verdict {
            action: Action::Pass,
            rule: "none",
            detail: serde_json::json!({}),
        }
    }

    fn hold(rule: &'static str, detail: serde_json::Value) -> Self {
        Verdict {
            action: Action::Hold,
            rule,
            detail,
        }
    }

    fn deny(rule: &'static str, detail: serde_json::Value) -> Self {
        Verdict {
            action: Action::Deny,
            rule,
            detail,
        }
    }
}

/// Thresholds.
///
/// Production should read these from config or campaign settings — numbers need tuning on real
/// data, not code releases. Constants here are defaults only.
pub mod thresholds {
    pub const MAX_PLAYERS_PER_DEVICE: i64 = 3;
    pub const MAX_REDEEM_PER_IP_DAY: i64 = 10;
    /// tg_user_id above this is treated as a newly registered account. TG user IDs roughly increase
    /// with registration time — a crude zero-cost signal; recalibrate periodically on live data.
    pub const NEW_ACCOUNT_TG_USER_ID: i64 = 7_500_000_000;
    /// Durations below this look like automation.
    pub const MIN_CLICK_TO_REDEEM_MS: i64 = 1_500;
}

/// L1 check input before play.
#[derive(Debug, Default)]
pub struct PlayInput {
    pub today_play_count: i64,
    pub daily_play_limit: i64,
}

/// Hard constraints before play.
///
/// Play directly consumes prize-pool cost and retrying is harmless to real users, so deny outright here.
pub fn check_play(input: &PlayInput) -> Verdict {
    if input.daily_play_limit > 0 && input.today_play_count >= input.daily_play_limit {
        return Verdict::deny(
            "daily_play_limit",
            serde_json::json!({
                "count": input.today_play_count,
                "limit": input.daily_play_limit,
            }),
        );
    }
    Verdict::pass()
}

/// L1 check input at redemption.
#[derive(Debug, Default)]
pub struct RedeemInput {
    /// Number of players already bound to this device_id
    pub device_player_count: i64,
    /// Redemptions from this IP today
    pub ip_redeem_today: i64,
    /// Rough account-age signal
    pub tg_user_id: i64,
    /// Total time from click to redemption. `None` means signal missing — do not treat as anomalous.
    pub click_to_redeem: Option<Duration>,
}

/// Hard constraints at redemption.
///
/// **Key trade-off: prefer hold over deny.**
///
/// Redemption is the end of the user journey. A false deny means no reward plus a first impression
/// that "this scammy promo doesn't work" — irreversible. Letting a bot through is temporary extra
/// counting; holds can reverse within the hold period or be rejected manually before money moves.
///
/// Only device farming is denied outright — many accounts on one device is unambiguous, and real
/// users almost never hit it.
pub fn check_redeem(input: &RedeemInput) -> Verdict {
    if input.device_player_count >= thresholds::MAX_PLAYERS_PER_DEVICE {
        return Verdict::deny(
            "device_player_limit",
            serde_json::json!({
                "count": input.device_player_count,
                "limit": thresholds::MAX_PLAYERS_PER_DEVICE,
            }),
        );
    }
    if input.ip_redeem_today >= thresholds::MAX_REDEEM_PER_IP_DAY {
        return Verdict::hold(
            "ip_redeem_rate",
            serde_json::json!({
                "count": input.ip_redeem_today,
                "limit": thresholds::MAX_REDEEM_PER_IP_DAY,
            }),
        );
    }
    if let Some(elapsed) = input.click_to_redeem
        && elapsed.num_milliseconds() < thresholds::MIN_CLICK_TO_REDEEM_MS
    {
        return Verdict::hold(
            "too_fast",
            serde_json::json!({ "elapsed_ms": elapsed.num_milliseconds() }),
        );
    }
    if input.tg_user_id > thresholds::NEW_ACCOUNT_TG_USER_ID {
        return Verdict::hold(
            "new_tg_account",
            serde_json::json!({ "tg_user_id": input.tg_user_id }),
        );
    }
    Verdict::pass()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_play_enforces_daily_limit() {
        let v = check_play(&PlayInput {
            today_play_count: 3,
            daily_play_limit: 3,
        });
        assert_eq!(v.action, Action::Deny);
        assert_eq!(v.rule, "daily_play_limit");

        let v = check_play(&PlayInput {
            today_play_count: 2,
            daily_play_limit: 3,
        });
        assert_eq!(v.action, Action::Pass);
    }

    /// Core redemption trade-off: prefer hold over deny. False deny is irreversible; false pass is
    /// reversible within the hold window.
    #[test]
    fn check_redeem_prefers_hold_over_deny() {
        let cases: [(&str, RedeemInput, &str); 3] = [
            (
                "IP redemption rate",
                RedeemInput {
                    ip_redeem_today: 10,
                    ..Default::default()
                },
                "ip_redeem_rate",
            ),
            (
                "elapsed too short",
                RedeemInput {
                    click_to_redeem: Some(Duration::milliseconds(500)),
                    ..Default::default()
                },
                "too_fast",
            ),
            (
                "newly registered account",
                RedeemInput {
                    tg_user_id: thresholds::NEW_ACCOUNT_TG_USER_ID + 1,
                    ..Default::default()
                },
                "new_tg_account",
            ),
        ];
        for (name, input, rule) in cases {
            let v = check_redeem(&input);
            assert_eq!(v.action, Action::Hold, "{name}: should hold, not deny");
            assert_eq!(v.rule, rule, "{name}");
        }
    }

    /// Only dimension denied outright: many accounts on one device is clear farming.
    #[test]
    fn check_redeem_denies_device_farming() {
        let v = check_redeem(&RedeemInput {
            device_player_count: thresholds::MAX_PLAYERS_PER_DEVICE,
            ..Default::default()
        });

        assert_eq!(v.action, Action::Deny);
        assert_eq!(v.rule, "device_player_limit");
    }

    #[test]
    fn check_redeem_passes_clean_request() {
        let v = check_redeem(&RedeemInput {
            device_player_count: 1,
            ip_redeem_today: 2,
            tg_user_id: 123_456_789,
            click_to_redeem: Some(Duration::seconds(45)),
        });

        assert_eq!(v.action, Action::Pass);
    }

    /// Missing latency signal (legacy data, not collected) must not count as an anomaly.
    #[test]
    fn check_redeem_ignores_missing_latency() {
        let v = check_redeem(&RedeemInput {
            tg_user_id: 123_456_789,
            click_to_redeem: None,
            ..Default::default()
        });

        assert_eq!(v.action, Action::Pass);
    }
}
