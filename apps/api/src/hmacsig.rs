//! Main-app postback (S2S) signing and verification.
//!
//! The postback endpoint is the only customer-initiated entry that can produce billable events;
//! it must prevent forgery and replay.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::telegram::constant_time_eq;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SigError {
    #[error("hmacsig: signature mismatch")]
    BadSignature,
    #[error("hmacsig: timestamp outside allowed window")]
    Stale,
    #[error("hmacsig: invalid timestamp format")]
    BadTimestamp,
}

/// Allowed clock skew window.
///
/// Five minutes each way: customer server clocks may drift from ours. Too narrow causes false
/// rejects ("missing postbacks", hurting revenue stats); too wide enlarges the replay window.
pub const DEFAULT_SKEW: TimeDelta = TimeDelta::minutes(5);

/// Compute signature: `HMAC-SHA256(secret, timestamp + "." + body)`.
///
/// Timestamp is in the signed material so it cannot be altered — otherwise attackers could replay
/// old requests with a new timestamp indefinitely.
pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts keys of any length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Verify signature and timestamp window.
///
/// Blocks half of replay via the time window; the other half is the `(tenant_id, event_type,
/// external_id)` unique constraint on `billable_event` — replays inside the window are absorbed
/// idempotently without a second charge. Both halves are required.
pub fn verify(
    secret: &[u8],
    timestamp_header: &str,
    signature: &str,
    body: &[u8],
    skew: TimeDelta,
    now: DateTime<Utc>,
) -> Result<(), SigError> {
    let ts: i64 = timestamp_header
        .parse()
        .map_err(|_| SigError::BadTimestamp)?;
    let sent_at = Utc
        .timestamp_opt(ts, 0)
        .single()
        .ok_or(SigError::BadTimestamp)?;

    let drift = now - sent_at;
    if drift > skew || drift < -skew {
        return Err(SigError::Stale);
    }

    let expected = sign(secret, ts, body);
    if !constant_time_eq(expected.as_bytes(), signature.as_bytes()) {
        return Err(SigError::BadSignature);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"tenant-postback-secret";

    #[test]
    fn accepts_valid() {
        let body = br#"{"app_user_id":"u1","transaction_id":"t1","amount":999}"#;
        let now = Utc::now();
        let sig = sign(SECRET, now.timestamp(), body);

        verify(
            SECRET,
            &now.timestamp().to_string(),
            &sig,
            body,
            DEFAULT_SKEW,
            now,
        )
        .expect("verification should pass");
    }

    #[test]
    fn rejects_tampered_body() {
        let now = Utc::now();
        let sig = sign(SECRET, now.timestamp(), br#"{"amount":999}"#);

        let err = verify(
            SECRET,
            &now.timestamp().to_string(),
            &sig,
            br#"{"amount":99900}"#,
            DEFAULT_SKEW,
            now,
        )
        .unwrap_err();

        assert_eq!(err, SigError::BadSignature);
    }

    #[test]
    fn rejects_wrong_secret() {
        let body = br#"{"amount":999}"#;
        let now = Utc::now();
        let sig = sign(SECRET, now.timestamp(), body);

        let err = verify(
            b"other-tenant-secret",
            &now.timestamp().to_string(),
            &sig,
            body,
            DEFAULT_SKEW,
            now,
        )
        .unwrap_err();

        assert_eq!(err, SigError::BadSignature);
    }

    /// Timestamp is signed — changing it breaks verification; attackers cannot refresh old replays.
    #[test]
    fn rejects_tampered_timestamp() {
        let body = br#"{"amount":999}"#;
        let now = Utc::now();
        let sig = sign(SECRET, now.timestamp(), body);

        let err = verify(
            SECRET,
            &(now.timestamp() + 1).to_string(),
            &sig,
            body,
            DEFAULT_SKEW,
            now,
        )
        .unwrap_err();

        assert_eq!(err, SigError::BadSignature);
    }

    #[test]
    fn rejects_stale() {
        let body = br#"{"amount":999}"#;
        let now = Utc::now();
        let sent = now - TimeDelta::minutes(30);
        let sig = sign(SECRET, sent.timestamp(), body);

        let err = verify(
            SECRET,
            &sent.timestamp().to_string(),
            &sig,
            body,
            DEFAULT_SKEW,
            now,
        )
        .unwrap_err();

        assert_eq!(err, SigError::Stale);
    }

    /// Window is bidirectional: reject clocks far in the future too — otherwise attackers sign
    /// future timestamps for an extended validity window.
    #[test]
    fn rejects_future_beyond_skew() {
        let body = br#"{"amount":999}"#;
        let now = Utc::now();
        let sent = now + TimeDelta::minutes(30);
        let sig = sign(SECRET, sent.timestamp(), body);

        let err = verify(
            SECRET,
            &sent.timestamp().to_string(),
            &sig,
            body,
            DEFAULT_SKEW,
            now,
        )
        .unwrap_err();

        assert_eq!(err, SigError::Stale);
    }

    #[test]
    fn rejects_bad_timestamp_format() {
        let err = verify(
            SECRET,
            "not-a-number",
            "deadbeef",
            b"{}",
            DEFAULT_SKEW,
            Utc::now(),
        )
        .unwrap_err();

        assert_eq!(err, SigError::BadTimestamp);
    }
}
