//! Attribution: policy versions, claim-code issuance and redemption, monetisation postback.

pub mod claim_code;
pub mod issue;
pub mod nondet;
pub mod policy;
pub mod postback;
pub mod query;
pub mod redeem;

pub use policy::by_version;
pub use redeem::{RedeemError, RedeemRequest, Service};
