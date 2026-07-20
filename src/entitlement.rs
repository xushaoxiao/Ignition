//! 能力门控（设计约束 C4）。
//!
//! **不允许 `if plan == "pro"`。** 这不是洁癖：早期销售一定会承诺「先免费给你
//! 开 Discord」「给你多开两个群」。硬编码的档位判断接不住这类承诺，接住的方式
//! 只有两条 —— 要么加个假的 plan，要么在代码里塞例外，三个月后都没人说得清
//! 某个客户到底买了什么。
//!
//! 所以能力由数据决定：`plan_entitlement` 给档位的缺省，
//! `tenant_entitlement_override` 给销售谈判的例外，后者覆盖前者且可带到期时间。

// 能力集本身已完整实现并有测试，但具体的门控点（明细导出、Discord 渠道、
// 分层分析）都还没落地，所以暂时没有调用方。等第一个付费能力接进来时移除。
#![allow(dead_code)]

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::Value;

/// entitlement 的 key。集中定义，避免同一个能力在两处写成不同的字符串 ——
/// 那种拼写差异的表现是「功能静默不生效」，排查起来极费时间。
pub mod key {
    /// 群组数量上限
    pub const CHANNEL_COUNT: &str = "channel.count";
    /// Discord 扩展
    pub const CHANNEL_DISCORD: &str = "channel.discord";
    /// WhatsApp 扩展
    pub const CHANNEL_WHATSAPP: &str = "channel.whatsapp";
    /// 专属模板
    pub const TEMPLATE_CUSTOM: &str = "template.custom";
    /// 三指标看板
    pub const ANALYTICS_BASIC: &str = "analytics.basic";
    /// 分层 / 留存分析
    pub const ANALYTICS_COHORT: &str = "analytics.cohort";
    /// 白标
    pub const BRANDING_WHITELABEL: &str = "branding.whitelabel";
    /// KOL 撮合
    pub const MARKETPLACE_KOL: &str = "marketplace.kol";
    /// 是否启用效果分成计费
    pub const BILLING_PERFORMANCE: &str = "billing.performance";
    /// 明细导出
    pub const EXPORT_RAW: &str = "export.raw";
}

/// 一个租户当前生效的全部能力。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entitlements {
    values: HashMap<String, Value>,
}

/// 一条 entitlement 记录，来自 plan 缺省或租户 override。
#[derive(Debug, Clone)]
pub struct Grant {
    pub key: String,
    pub value: Value,
    /// 仅 override 有到期时间；plan 缺省恒为 `None`。
    pub expires_at: Option<DateTime<Utc>>,
}

impl Entitlements {
    /// 把 plan 缺省与租户 override 合成为当前生效的能力集。
    ///
    /// override 后应用，因此覆盖同名的 plan 缺省。已过期的 override 直接丢弃
    /// 而**不是**回退成「关闭」—— 丢弃后 plan 缺省会重新生效，这才是
    /// 「临时多给你开一个月」到期后应有的行为。
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

    /// 布尔能力是否开启。
    ///
    /// **缺省关闭**：查不到的 key 一律视为没有。新增一项付费能力时，忘了给
    /// 任何 plan 配上它，结果是「所有人都用不了」而不是「所有人都免费用」——
    /// 前者会有人来报，后者没人会报。
    pub fn check(&self, key: &str) -> bool {
        match self.values.get(key) {
            Some(Value::Bool(b)) => *b,
            // {"limit": n} 形态的能力，配了就算开启（n 由 limit() 取）
            Some(Value::Object(o)) => o.contains_key("limit"),
            _ => false,
        }
    }

    /// 取数量型能力的上限。`None` 表示未配置，调用方应按「不允许」处理。
    pub fn limit(&self, key: &str) -> Option<i64> {
        self.values.get(key)?.get("limit")?.as_i64()
    }

    /// 当前用量是否还在上限内。未配置上限时返回 false（缺省关闭）。
    pub fn within_limit(&self, key: &str, current: i64) -> bool {
        self.limit(key).is_some_and(|max| current < max)
    }
}

// ---------------------------------------------------------------- 订阅状态

/// 订阅状态对应的服务等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceLevel {
    /// 完整服务
    Full,
    /// 只读：看板可看、数据可导，但链接停止分发、游戏不再受理
    ReadOnly,
}

/// 由订阅状态推导服务等级。
///
/// **`past_due` 在宽限期内不降级。** 断服会让 KOL 侧的链路当场失效，损害的是
/// 客户的客户 —— 客户的反应是流失，不是补款。宽限期后降为只读而不是彻底关停，
/// 数据仍可导出，留住「回来续费」的可能。
pub fn service_level(
    status: SubscriptionStatus,
    grace_until: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> ServiceLevel {
    use SubscriptionStatus::*;
    match status {
        Trialing | Active => ServiceLevel::Full,
        // 宽限期未设置时按「已过期」处理：宁可保守，也不要因为漏写一个字段
        // 就无限期免费服务。
        PastDue => match grace_until {
            Some(t) if now < t => ServiceLevel::Full,
            _ => ServiceLevel::ReadOnly,
        },
        Paused | Canceled => ServiceLevel::ReadOnly,
    }
}

/// 订阅状态。取值与数据库 `subscription_status` 枚举一一对应。
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

    /// 缺省关闭：没配过的能力一律没有。配错 key 的表现应该是「用不了」，
    /// 不是「免费送」。
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

    /// 销售承诺「先免费给你开 Discord」的落地方式：一条 override，不是一行代码。
    #[test]
    fn override_beats_plan_default() {
        let e = Entitlements::resolve(
            vec![grant(key::CHANNEL_DISCORD, json!(false))],
            vec![grant(key::CHANNEL_DISCORD, json!(true))],
            now(),
        );
        assert!(e.check(key::CHANNEL_DISCORD));
    }

    /// 过期的 override 不是「关闭该能力」，而是「回落到 plan 缺省」。
    /// 「临时多给你开一个月」到期后应当回到他买的档位，而不是掉到零。
    #[test]
    fn expired_override_falls_back_to_plan_default() {
        let plan = vec![grant(key::CHANNEL_COUNT, json!({"limit": 1}))];
        let promo = vec![expiring(
            key::CHANNEL_COUNT,
            json!({"limit": 10}),
            now() - TimeDelta::days(1),
        )];

        let e = Entitlements::resolve(plan, promo, now());
        assert_eq!(e.limit(key::CHANNEL_COUNT), Some(1), "应回落到 plan 缺省");
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

    // ------------------------------------------------------------ 服务等级

    /// past_due 宽限期内不断服：断服损害的是客户的客户，客户会流失而不是补款。
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

    /// 漏写 grace_until 时按已过期处理 —— 保守方向是少给服务，不是无限期免费。
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
