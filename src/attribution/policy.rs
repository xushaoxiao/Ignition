//! 归因规则的版本化定义。

use chrono::TimeDelta;

use crate::models::event_type;

/// 归因规则的一个版本。
///
/// **这张表就是产品本身**：客户按它验算，KOL 按它申诉。规则变更必须发新版本号
/// 并提前通知，绝不静默修改（约束 C2）。每条 Attribution 记录都会存下当时的
/// `policy_version`，使得任何一笔历史账单都能被精确复算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub version: &'static str,
    /// 点击归因窗口：超过则该点击失效。
    pub click_window: TimeDelta,
    /// 领奖码有效期。
    pub claim_code_ttl: TimeDelta,
    /// 归因锁定期：期内该 Player 的转化都算原 KOL。
    pub lock_period: TimeDelta,
    /// activation 的冷静期，期内可冲正。
    pub activation_hold: TimeDelta,
    /// iap_purchase 的冷静期，需覆盖 App Store 退款窗口。
    pub purchase_hold: TimeDelta,
}

/// 当前生效的归因规则。对应 `docs/attribution-policy-v1.md`。
pub const V1: Policy = Policy {
    version: "v1",
    click_window: TimeDelta::days(7),
    claim_code_ttl: TimeDelta::hours(24),
    lock_period: TimeDelta::days(90),
    // activation 由我方核销确认，7 天足够覆盖异常发现窗口
    activation_hold: TimeDelta::days(7),
    // 未来的 GMV 分成需覆盖 App Store 退款窗口，故取 35 天
    purchase_hold: TimeDelta::days(35),
};

#[derive(Debug, thiserror::Error)]
#[error("attribution: 未知的归因规则版本 {0:?}")]
pub struct UnknownPolicyVersion(pub String);

/// 按版本号取回归因规则，用于申诉复算历史记录。
pub fn by_version(v: &str) -> Result<Policy, UnknownPolicyVersion> {
    match v {
        "v1" => Ok(V1),
        other => Err(UnknownPolicyVersion(other.to_string())),
    }
}

impl Policy {
    /// 某事件类型的冷静期。
    ///
    /// 未知类型回退到 7 天 —— 宁可多押一会儿也不要立刻计费。
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

    /// IAP 的冷静期必须显著长于 activation —— 它要覆盖 App Store 的退款窗口，
    /// 否则钱付给 KOL 之后才发现是退款订单。
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
