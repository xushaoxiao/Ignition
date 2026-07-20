//! 归因：规则版本、领奖码签发与核销、变现回传。

pub mod claim_code;
pub mod issue;
pub mod policy;
pub mod postback;
pub mod redeem;

pub use policy::by_version;
pub use redeem::{RedeemError, RedeemRequest, Service};
