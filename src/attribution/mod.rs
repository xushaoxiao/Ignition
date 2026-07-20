//! 归因：规则版本、领奖码、核销事务。

pub mod claim_code;
pub mod policy;
pub mod redeem;

pub use policy::by_version;
pub use redeem::{RedeemError, RedeemRequest, Service};
