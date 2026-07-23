//! TMA session tokens.
//!
//! End users enter via Telegram Mini App; identity comes from initData, valid only at open
//! (we cap freshness at 5 minutes — see `telegram::DEFAULT_MAX_AGE`). Later requests must
//! not replay it — replay means intercepted initData can impersonate the user indefinitely.
//! After verification, issue our own short-lived tokens immediately.
//!
//! Two token kinds:
//!
//! - **access**: 15 minutes, sent on every business request.
//! - **refresh**: 7 days, used only to obtain new access tokens.
//!
//! Why refresh exists: initData does not update during the page lifecycle, yet users may
//! background the mini app beyond 15 minutes. Without refresh, they return logged out — or
//! we lengthen access to hours, widening leak impact.
//!
//! **Known trade-off**: refresh is a stateless JWT — cannot revoke individually before expiry,
//! only by rotating the signing key. TMA sessions are low privilege (own game, own codes), so
//! acceptable; KOL console sessions must be stateful when that ships.

use chrono::{DateTime, TimeDelta, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

pub const ACCESS_TTL: TimeDelta = TimeDelta::minutes(15);
pub const REFRESH_TTL: TimeDelta = TimeDelta::days(7);

/// Token purpose.
///
/// Must live in claims and be checked on verify — otherwise a 7-day refresh token could be
/// used as access, silently extending access lifetime to 7 days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    Access,
    Refresh,
}

/// Session payload.
///
/// The four attribution IDs (campaign / link / kol / player) are fixed at token issuance;
/// business handlers must not read them from the client — otherwise the frontend could set
/// `kol_id` and attribute conversions to any KOL, destroying attribution trust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// player_id
    pub sub: i64,
    pub tenant_id: i64,
    pub campaign_id: i64,
    pub link_id: i64,
    pub kol_id: i64,
    pub kind: TokenKind,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum JwtError {
    #[error("jwt: token invalid or expired")]
    Invalid,
    #[error("jwt: token purpose mismatch")]
    WrongKind,
}

/// Token issuance and verification.
#[derive(Clone)]
pub struct Issuer {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl std::fmt::Debug for Issuer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Issuer(<signing key redacted>)")
    }
}

/// Token exchange result.
#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds until access expiry — lets the frontend refresh proactively instead of waiting for 401.
    pub expires_in: i64,
}

impl Issuer {
    pub fn new(signing_key: &[u8]) -> Self {
        Issuer {
            encoding: EncodingKey::from_secret(signing_key),
            decoding: DecodingKey::from_secret(signing_key),
        }
    }

    /// Issue an access/refresh token pair.
    pub fn issue(&self, session: &SessionSubject, now: DateTime<Utc>) -> Session {
        Session {
            access_token: self.sign(session, TokenKind::Access, ACCESS_TTL, now),
            refresh_token: self.sign(session, TokenKind::Refresh, REFRESH_TTL, now),
            expires_in: ACCESS_TTL.num_seconds(),
        }
    }

    fn sign(
        &self,
        s: &SessionSubject,
        kind: TokenKind,
        ttl: TimeDelta,
        now: DateTime<Utc>,
    ) -> String {
        let claims = Claims {
            sub: s.player_id,
            tenant_id: s.tenant_id,
            campaign_id: s.campaign_id,
            link_id: s.link_id,
            kol_id: s.kol_id,
            kind,
            iat: now.timestamp(),
            exp: (now + ttl).timestamp(),
        };
        // HS256 with fixed claims shape — encoding cannot fail here.
        jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .expect("JWT encoding should not fail")
    }

    /// Verify token and expected purpose.
    ///
    /// `now` is injected by callers rather than read from the real clock so tests cover expiry
    /// without sleep.
    pub fn verify(
        &self,
        token: &str,
        want: TokenKind,
        now: DateTime<Utc>,
    ) -> Result<Claims, JwtError> {
        let mut validation = Validation::new(Algorithm::HS256);
        // Expiry is checked against injected now; disable library clock-based validation.
        validation.validate_exp = false;
        validation.required_spec_claims.clear();

        let data = jsonwebtoken::decode::<Claims>(token, &self.decoding, &validation)
            .map_err(|_| JwtError::Invalid)?;
        let claims = data.claims;

        if now.timestamp() >= claims.exp {
            return Err(JwtError::Invalid);
        }
        if claims.kind != want {
            return Err(JwtError::WrongKind);
        }
        Ok(claims)
    }
}

/// Session subject required to issue tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSubject {
    pub tenant_id: i64,
    pub player_id: i64,
    pub campaign_id: i64,
    pub link_id: i64,
    pub kol_id: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 21, 12, 0, 0).unwrap()
    }

    fn subject() -> SessionSubject {
        SessionSubject {
            tenant_id: 1,
            player_id: 42,
            campaign_id: 7,
            link_id: 3,
            kol_id: 9,
        }
    }

    fn issuer() -> Issuer {
        Issuer::new(b"test-signing-key-test-signing-key")
    }

    #[test]
    fn round_trips_a_session() {
        let iss = issuer();
        let s = iss.issue(&subject(), now());

        let claims = iss
            .verify(&s.access_token, TokenKind::Access, now())
            .unwrap();
        assert_eq!(claims.sub, 42);
        assert_eq!(claims.tenant_id, 1);
        assert_eq!(claims.kol_id, 9);
        assert_eq!(s.expires_in, 900);
    }

    /// Refresh tokens must not pass as access — otherwise the 15-minute access TTL is meaningless.
    #[test]
    fn refresh_token_is_not_accepted_as_access() {
        let iss = issuer();
        let s = iss.issue(&subject(), now());

        let err = iss
            .verify(&s.refresh_token, TokenKind::Access, now())
            .unwrap_err();
        assert_eq!(err, JwtError::WrongKind);

        iss.verify(&s.refresh_token, TokenKind::Refresh, now())
            .expect("should pass when kind matches");
    }

    #[test]
    fn access_expires_after_its_ttl() {
        let iss = issuer();
        let s = iss.issue(&subject(), now());

        iss.verify(
            &s.access_token,
            TokenKind::Access,
            now() + TimeDelta::minutes(14),
        )
        .expect("should pass before expiry");

        let err = iss
            .verify(
                &s.access_token,
                TokenKind::Access,
                now() + TimeDelta::minutes(16),
            )
            .unwrap_err();
        assert_eq!(err, JwtError::Invalid);
    }

    #[test]
    fn refresh_outlives_access() {
        let iss = issuer();
        let s = iss.issue(&subject(), now());
        let later = now() + TimeDelta::days(6);

        assert!(
            iss.verify(&s.access_token, TokenKind::Access, later)
                .is_err()
        );
        assert!(
            iss.verify(&s.refresh_token, TokenKind::Refresh, later)
                .is_ok()
        );
    }

    /// Rotating the signing key must invalidate all tokens — the only revocation lever for stateless refresh.
    #[test]
    fn tokens_signed_by_another_key_are_rejected() {
        let s = issuer().issue(&subject(), now());
        let other = Issuer::new(b"a-completely-different-signing-key");

        assert_eq!(
            other
                .verify(&s.access_token, TokenKind::Access, now())
                .unwrap_err(),
            JwtError::Invalid
        );
    }

    #[test]
    fn tampered_tokens_are_rejected() {
        let iss = issuer();
        let s = iss.issue(&subject(), now());

        // Change one character in the payload segment — signature no longer matches.
        let mut parts: Vec<&str> = s.access_token.split('.').collect();
        let payload = parts[1].to_string();
        let swapped = format!("X{}", &payload[1..]);
        parts[1] = &swapped;
        let tampered = parts.join(".");

        assert_eq!(
            iss.verify(&tampered, TokenKind::Access, now()).unwrap_err(),
            JwtError::Invalid
        );
    }

    #[test]
    fn garbage_is_rejected() {
        let iss = issuer();
        for bad in ["", "not-a-jwt", "a.b.c"] {
            assert_eq!(
                iss.verify(bad, TokenKind::Access, now()).unwrap_err(),
                JwtError::Invalid
            );
        }
    }

    /// `alg: none` attack when signing key is compromised — library pins HS256; this is a second guard.
    #[test]
    fn unsigned_tokens_are_rejected() {
        let iss = issuer();
        let header = base64_url(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = base64_url(
            br#"{"sub":1,"tenant_id":1,"campaign_id":1,"link_id":1,"kol_id":1,"kind":"access","iat":0,"exp":99999999999}"#,
        );
        let token = format!("{header}.{payload}.");

        assert_eq!(
            iss.verify(&token, TokenKind::Access, now()).unwrap_err(),
            JwtError::Invalid
        );
    }

    fn base64_url(b: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
    }
}
