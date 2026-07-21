//! API Key + HMAC request signing (S2S).
//!
//! Replaces the previous placeholder that read tenant from the `X-Tenant-ID` header. That meant
//! anyone who guessed a tenant ID and a claim code could redeem someone else's codes or forge
//! billing — revenue correctness requires the caller to actually be the tenant they claim.
//!
//! Request headers:
//!
//! ```text
//! X-Ignition-Key:       ik_live_xxx        Key identifier, plaintext
//! X-Ignition-Timestamp: 1753056000         Unix seconds
//! X-Ignition-Signature: <hex>              HMAC-SHA256
//! ```
//!
//! The signed object is the **canonical request**, not the body alone — see [`canonical_request`].

use chrono::{DateTime, TimeDelta, Utc};

use crate::hmacsig::{self, SigError};

pub const HEADER_KEY: &str = "X-Ignition-Key";
pub const HEADER_TIMESTAMP: &str = "X-Ignition-Timestamp";
pub const HEADER_SIGNATURE: &str = "X-Ignition-Signature";

/// Allowed clock skew — same value as the postback endpoint.
pub const DEFAULT_SKEW: TimeDelta = hmacsig::DEFAULT_SKEW;

/// API key permission scopes.
///
/// Parsed with exhaustive `match` rather than raw string compare: adding a capability forces
/// registration here — no "scope in DB but unknown in code, silently denied" drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Redeem claim codes
    Redeem,
    /// Monetisation postback
    Postback,
    /// Read attribution
    ReadAttribution,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Redeem => "redeem",
            Scope::Postback => "postback",
            Scope::ReadAttribution => "attribution:read",
        }
    }

    pub fn parse(s: &str) -> Option<Scope> {
        match s {
            "redeem" => Some(Scope::Redeem),
            "postback" => Some(Scope::Postback),
            "attribution:read" => Some(Scope::ReadAttribution),
            _ => None,
        }
    }
}

/// Build the canonical signed request: `METHOD\npath\nbody`.
///
/// **Method and path must be in the signed material.** Otherwise a signature valid for
/// `/v1/claims/redeem` could be replayed to `/v1/postback/purchase` — different bodies, but if
/// only the body is signed, an attacker can pick which endpoint to hit. Sign the whole request,
/// not just the byte string.
pub fn canonical_request(method: &str, path: &str, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(method.len() + path.len() + body.len() + 2);
    out.extend_from_slice(method.to_ascii_uppercase().as_bytes());
    out.push(b'\n');
    out.extend_from_slice(path.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(body);
    out
}

/// Compute a request signature.
///
/// The server verifies only — this has no online call sites. It exists for integration docs,
/// client SDK reference, and tests building valid requests. Lives in code, not docs, so the
/// algorithm has a single definition: doc typos go unnoticed; code typos fail tests.
#[allow(dead_code)]
pub fn sign(secret: &[u8], timestamp: i64, method: &str, path: &str, body: &[u8]) -> String {
    hmacsig::sign(secret, timestamp, &canonical_request(method, path, body))
}

/// Verify request signature and freshness.
///
/// The time window blocks half of replay; the other half is write-path idempotency
/// (`billable_event` unique keys, `claim_code` state machine). Both are required: window alone
/// allows replay inside the window; idempotency alone allows indefinite replay with the same response
/// to probe system state.
#[allow(clippy::too_many_arguments)]
pub fn verify(
    secret: &[u8],
    method: &str,
    path: &str,
    body: &[u8],
    timestamp_header: &str,
    signature: &str,
    skew: TimeDelta,
    now: DateTime<Utc>,
) -> Result<(), SigError> {
    let canonical = canonical_request(method, path, body);
    hmacsig::verify(secret, timestamp_header, signature, &canonical, skew, now)
}

/// Check whether a key has a given scope.
///
/// Default deny: scopes not listed are absent. Main-app keys typically need only `redeem` — even
/// if the key leaks, attackers cannot forge monetisation postbacks.
pub fn has_scope(scopes: &[String], want: Scope) -> bool {
    scopes
        .iter()
        .filter_map(|s| Scope::parse(s))
        .any(|s| s == want)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"tenant-api-secret";
    const BODY: &[u8] = br#"{"claim_code":"DEMA2345"}"#;

    fn now() -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 7, 21, 12, 0, 0).unwrap()
    }

    fn verify_at(method: &str, path: &str, body: &[u8], sig: &str) -> Result<(), SigError> {
        verify(
            SECRET,
            method,
            path,
            body,
            &now().timestamp().to_string(),
            sig,
            DEFAULT_SKEW,
            now(),
        )
    }

    #[test]
    fn accepts_a_correctly_signed_request() {
        let sig = sign(SECRET, now().timestamp(), "POST", "/v1/claims/redeem", BODY);
        verify_at("POST", "/v1/claims/redeem", BODY, &sig).expect("verification should pass");
    }

    /// Signature must bind the path — otherwise a redeem signature could be replayed to postback,
    /// turning a redeem-only key into billable monetisation events.
    #[test]
    fn signature_is_bound_to_the_path() {
        let sig = sign(SECRET, now().timestamp(), "POST", "/v1/claims/redeem", BODY);

        let err = verify_at("POST", "/v1/postback/purchase", BODY, &sig).unwrap_err();
        assert_eq!(err, SigError::BadSignature);
    }

    #[test]
    fn signature_is_bound_to_the_method() {
        let sig = sign(SECRET, now().timestamp(), "POST", "/v1/claims/redeem", BODY);

        let err = verify_at("GET", "/v1/claims/redeem", BODY, &sig).unwrap_err();
        assert_eq!(err, SigError::BadSignature);
    }

    #[test]
    fn signature_is_bound_to_the_body() {
        let sig = sign(SECRET, now().timestamp(), "POST", "/v1/claims/redeem", BODY);

        let err = verify_at(
            "POST",
            "/v1/claims/redeem",
            br#"{"claim_code":"OTHER123"}"#,
            &sig,
        )
        .unwrap_err();
        assert_eq!(err, SigError::BadSignature);
    }

    #[test]
    fn rejects_a_stale_timestamp() {
        let old = now() - TimeDelta::minutes(30);
        let sig = sign(SECRET, old.timestamp(), "POST", "/v1/claims/redeem", BODY);

        let err = verify(
            SECRET,
            "POST",
            "/v1/claims/redeem",
            BODY,
            &old.timestamp().to_string(),
            &sig,
            DEFAULT_SKEW,
            now(),
        )
        .unwrap_err();
        assert_eq!(err, SigError::Stale);
    }

    #[test]
    fn method_case_does_not_matter() {
        let a = sign(SECRET, now().timestamp(), "post", "/v1/x", BODY);
        let b = sign(SECRET, now().timestamp(), "POST", "/v1/x", BODY);
        assert_eq!(a, b);
    }

    /// Default deny: scopes not in the database must not be accidentally granted.
    #[test]
    fn scopes_default_to_denied() {
        let only_redeem = vec!["redeem".to_string()];
        assert!(has_scope(&only_redeem, Scope::Redeem));
        assert!(!has_scope(&only_redeem, Scope::Postback));
        assert!(!has_scope(&[], Scope::Redeem));
    }

    /// Unrecognised scope strings must grant nothing — typos should look like "no permission",
    /// not accidental matches elsewhere.
    #[test]
    fn unknown_scope_strings_grant_nothing() {
        let typo = vec!["redeeem".to_string(), "*".to_string()];
        for want in [Scope::Redeem, Scope::Postback, Scope::ReadAttribution] {
            assert!(!has_scope(&typo, want), "{want:?} should not be granted");
        }
    }

    #[test]
    fn scope_strings_round_trip() {
        for s in [Scope::Redeem, Scope::Postback, Scope::ReadAttribution] {
            assert_eq!(Scope::parse(s.as_str()), Some(s));
        }
    }
}
