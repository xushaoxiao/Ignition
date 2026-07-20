//! 风控的 L1 硬约束与 L2 信号定义。
//!
//! 风控在这个系统里有双重身份：既保护奖励成本，也保护计费准确性。后者更重要
//! —— 一条被判定为作弊的转化，如果已经收了客户的钱，损害的是信任而不只是钱。

use chrono::Duration;
use serde::{Deserialize, Serialize};

/// L1 规则的处置动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Pass,
    /// 标记暂缓：事件照常记录，但不进账单，等人工复核
    Hold,
    Deny,
}

/// 一次风控判定。
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

/// 阈值。
///
/// 生产环境应从配置或 campaign 上读取 —— 这些数字需要按真实数据调，
/// 而不是靠改代码发版。这里的常量只是缺省值。
pub mod thresholds {
    pub const MAX_PLAYERS_PER_DEVICE: i64 = 3;
    pub const MAX_REDEEM_PER_IP_DAY: i64 = 10;
    /// 超过此值的 tg_user_id 视为新注册账号。TG user_id 大体随注册时间
    /// 递增，这是个粗糙但零成本的信号，需要定期按实际数据校准。
    pub const NEW_ACCOUNT_TG_USER_ID: i64 = 7_500_000_000;
    /// 低于此耗时视为脚本特征。
    pub const MIN_CLICK_TO_REDEEM_MS: i64 = 1_500;
}

/// 抽奖前的 L1 检查输入。
#[derive(Debug, Default)]
pub struct PlayInput {
    pub today_play_count: i64,
    pub daily_play_limit: i64,
}

/// 抽奖前的硬约束。
///
/// 抽奖直接消耗奖池成本，且重来一次对真实用户没有损失，所以这里可以直接拒绝。
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

/// 核销时的 L1 检查输入。
#[derive(Debug, Default)]
pub struct RedeemInput {
    /// 该 device_id 已绑定的 Player 数
    pub device_player_count: i64,
    /// 该 IP 今日核销数
    pub ip_redeem_today: i64,
    /// 用于粗判账号年龄
    pub tg_user_id: i64,
    /// 从点击到核销的总耗时。`None` 表示信号缺失，不应据此判定异常。
    pub click_to_redeem: Option<Duration>,
}

/// 核销时的硬约束。
///
/// **关键取舍：这里尽量只 hold 不 deny。**
///
/// 核销是用户旅程的终点，误杀一个真实用户 = 他领不到奖 + 对客户 App 的第一
/// 印象是「这破活动是骗人的」，这个损失不可挽回。而放过一个刷子只是暂时多算
/// 一笔，冷静期内可以冲正、可以人工驳回，钱还没真正付出去。
///
/// 唯一直接拒绝的是设备维度 —— 一台设备绑定过多账号是明确的养号特征，
/// 且真实用户几乎不可能触发。
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
    if let Some(elapsed) = input.click_to_redeem {
        if elapsed.num_milliseconds() < thresholds::MIN_CLICK_TO_REDEEM_MS {
            return Verdict::hold(
                "too_fast",
                serde_json::json!({ "elapsed_ms": elapsed.num_milliseconds() }),
            );
        }
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

    /// 核销环节的核心取舍：尽量只 hold 不 deny。误杀一个真实用户不可挽回；
    /// 放过一个刷子只是暂时多算一笔，冷静期内可冲正。
    #[test]
    fn check_redeem_prefers_hold_over_deny() {
        let cases: [(&str, RedeemInput, &str); 3] = [
            (
                "IP 核销过频",
                RedeemInput {
                    ip_redeem_today: 10,
                    ..Default::default()
                },
                "ip_redeem_rate",
            ),
            (
                "耗时过短",
                RedeemInput {
                    click_to_redeem: Some(Duration::milliseconds(500)),
                    ..Default::default()
                },
                "too_fast",
            ),
            (
                "新注册账号",
                RedeemInput {
                    tg_user_id: thresholds::NEW_ACCOUNT_TG_USER_ID + 1,
                    ..Default::default()
                },
                "new_tg_account",
            ),
        ];
        for (name, input, rule) in cases {
            let v = check_redeem(&input);
            assert_eq!(v.action, Action::Hold, "{name}: 应暂缓而非拒绝");
            assert_eq!(v.rule, rule, "{name}");
        }
    }

    /// 唯一直接拒绝的维度：一台设备绑定过多账号是明确的养号特征。
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

    /// 耗时信号缺失（老数据、未采集）不应被当成异常。
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
