//! Caller identity.
//!
//! The system has two caller types with different auth because the threat models differ:
//!
//! | Caller | Method | Why |
//! |---|---|---|
//! | Customer main-app backend (S2S) | API Key + HMAC request signature | Server can hold long-lived secrets; signature prevents tampering and replay |
//! | TMA frontend (end-user browser) | Short-lived JWT after initData exchange | Frontend cannot hold long-lived secrets — only short-lived, low-privilege credentials |
//!
//! API Key instead of OAuth is deliberate: integration cost drives sales friction; one fewer
//! authorisation round-trip is one fewer week on the customer's schedule (design doc §10).

pub mod apikey;
pub mod jwt;

pub use apikey::Scope;

/// Authenticated S2S caller identity.
///
/// All S2S handlers take `tenant_id` from here — nowhere reads tenant from the body or
/// custom headers anymore. That was the placeholder this work replaced.
///
/// TMA identity uses [`jwt::Claims`] instead: different credentials, lifetimes, and permission
/// models. Merging into one type only blurs "who actually sent this request".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    pub tenant_id: i64,
    pub api_key_id: i64,
}
