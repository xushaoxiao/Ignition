//! 领域类型与跨模块共享的枚举。
//!
//! 这里承载设计文档 §3 的领域模型，以及两条不可违反的规则：
//!
//! - 归因方式与「是否可计费」的映射（约束 C1）
//! - 可计费事件的状态机（约束 C3）
//!
//! 两者都用穷尽 `match` 实现而非查表。这是选 Rust 的直接收益：新增一种归因
//! 方式时，编译器会强制你在 `is_billable` 里明确表态，不可能因为忘了登记而
//! 默认落到某个分支。计费口径不该靠人记得去维护一张表。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- 金额

/// 金额，单位为最小货币单位（cent）。
///
/// 用 newtype 而非裸 `i64`：系统里同时存在数量、ID、时长等一堆 i64，
/// 把它们和金额混起来是最容易发生且最难发现的一类错误。
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

// ---------------------------------------------------------------- 归因

/// 归因方式。取值与数据库 `attribution_method` 枚举一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "attribution_method", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AttributionMethod {
    /// 领奖码核销 —— iOS 上唯一可计费的路径
    DeterministicCode,
    /// Android Play Install Referrer，用户零操作
    InstallReferrer,
    /// 已安装用户直接唤起，参数在 URL 里
    UniversalLink,
    /// 剪贴板匹配 —— 可提升看板转化率，但不进账单
    ClipboardMatch,
    /// 指纹 / 时间窗匹配 —— iOS 17+ 精度已崩塌，仅统计
    Probabilistic,
}

impl AttributionMethod {
    /// 该归因方式是否可计费。
    ///
    /// **这是整个商业模型的地基。** 中层效果分成只对确定性归因收费；概率归因
    /// 的转化可以进看板，绝不能进账单（约束 C1）。
    ///
    /// 背景：iOS 17+ 之后 user-level 的 deferred deep link 已无可靠实现 ——
    /// IDFA 拿不到，指纹匹配精度崩塌，AdAttributionKit 只给聚合且延迟的回传。
    /// 按概率匹配计费，等于向客户收一笔我们自己也无法验证的钱。
    pub fn is_billable(self) -> bool {
        use AttributionMethod::*;
        match self {
            DeterministicCode | InstallReferrer | UniversalLink => true,
            ClipboardMatch | Probabilistic => false,
        }
    }

    /// 置信度，用于看板分层展示。
    pub fn confidence(self) -> i16 {
        use AttributionMethod::*;
        match self {
            DeterministicCode | InstallReferrer | UniversalLink => 100,
            ClipboardMatch => 60,
            Probabilistic => 30,
        }
    }
}

/// 归因记录，系统的信任基石。
///
/// `evidence` 存判定当时的完整输入快照，是 KOL 申诉时唯一的证据来源，只增不改。
/// `policy_version` 对应一份对客户公开的规则文档，规则变更必须发新版本号。
// TODO(query): 归因查询接口（GET /v1/attribution/:app_user_id）与申诉复算
// 落地后会读它，届时移除 allow。
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
    /// 冗余字段（逻辑上可由 `method` 推导），但必须存：计费规则会变，
    /// 而已开出账单的判定依据必须冻结。
    pub is_billable: bool,
    pub policy_version: String,
    pub touch_at: DateTime<Utc>,
    pub attributed_at: DateTime<Utc>,
    pub locked_until: DateTime<Utc>,
    pub evidence: serde_json::Value,
}

// ---------------------------------------------------------------- 计费

/// 可计费事件的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "billable_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum BillableStatus {
    /// 已接收，在 hold 冷静期内
    Pending,
    /// 风控暂缓，等待人工复核
    Held,
    /// 已放行，可计入账单
    Cleared,
    /// 已开票
    Billed,
    /// 已冲正（退款 / 事后判定作弊）
    Reversed,
    /// 判定无效，不计费
    Rejected,
}

impl BillableStatus {
    /// 能否从当前状态迁移到 `to`。
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
    /// `Billed` 之后仍可 `Reversed` —— 冲正走下个账期的 credit，
    /// 不追溯修改已出的账单。
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

/// 非法的状态迁移。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("可计费事件: 非法状态迁移 {from:?} -> {to:?}")]
pub struct IllegalTransition {
    pub from: BillableStatus,
    pub to: BillableStatus,
}

/// 事件类型。
pub mod event_type {
    pub const ACTIVATION: &str = "activation";
    pub const IAP_PURCHASE: &str = "iap_purchase";
}

/// 可计费事件，收入的原子。
///
/// 只有 `Attribution::is_billable` 为 true 的转化才会产生 `BillableEvent`；
/// 不可计费的转化只进分析流（ClickHouse），不进这张表。
// 字段与表结构一一对应，`FromRow` 整行读出。其中 external_id、occurred_at
// 等几项的消费方是明细导出与差异视图，那两块还没写。
#[allow(dead_code)]
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BillableEvent {
    pub id: i64,
    pub tenant_id: i64,
    pub attribution_id: i64,
    pub event_type: String,
    /// 主 App 侧唯一 ID，幂等键
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

// ---------------------------------------------------------------- 账本

/// 账本科目。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Account {
    /// 客户应付我方
    TenantReceivable,
    /// 平台收入
    PlatformRevenue,
    /// 我方应付 KOL
    KolPayable,
    /// 冲正过渡
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

/// 借贷方向。
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

    /// 与 `Txn::reverse` 成对，冲正链路落地后才有在线调用方。
    #[allow(dead_code)]
    pub fn flip(self) -> Direction {
        match self {
            Direction::Debit => Direction::Credit,
            Direction::Credit => Direction::Debit,
        }
    }
}

// ---------------------------------------------------------------- 其它

/// 领奖码状态。
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

    /// 这条测试守护整个商业模型的地基。如果有人不小心把 Probabilistic
    /// 改成可计费，账单立刻失去公信力。
    #[test]
    fn only_deterministic_methods_are_billable() {
        use AttributionMethod::*;
        for m in [DeterministicCode, InstallReferrer, UniversalLink] {
            assert!(m.is_billable(), "{m:?} 应可计费");
            assert_eq!(m.confidence(), 100, "{m:?} 置信度应为 100");
        }
        for m in [ClipboardMatch, Probabilistic] {
            assert!(!m.is_billable(), "{m:?} 不应计费");
            assert!(m.confidence() < 100, "{m:?} 置信度应低于 100");
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
            // 已开票仍可冲正 —— 走下个账期的 credit
            (Billed, Reversed),
        ];
        for (from, to) in legal {
            assert!(from.can_transition_to(to), "{from:?} -> {to:?} 应合法");
        }
    }

    #[test]
    fn billable_state_machine_rejects_illegal_transitions() {
        use BillableStatus::*;
        let illegal = [
            (Billed, Cleared),   // 不可回退重新放行
            (Reversed, Cleared), // 冲正是终态
            (Rejected, Cleared), // 驳回是终态
            (Pending, Billed),   // 必须先过冷静期
            (Held, Billed),      // 暂缓的不能直接开票
        ];
        for (from, to) in illegal {
            assert!(!from.can_transition_to(to), "{from:?} -> {to:?} 应非法");
        }
    }

    #[test]
    fn cents_displays_as_decimal() {
        assert_eq!(Cents(200).to_string(), "2.00");
        assert_eq!(Cents(9900).to_string(), "99.00");
        assert_eq!(Cents(5).to_string(), "0.05");
    }
}
