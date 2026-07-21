//! Telegram Mini App initData verification.
//!
//! This is the system's first trust boundary: initData determines which TG user sent
//! the request, and that identity is eventually bound to billable attribution via claim-code
//! redemption. If verification is bypassed, the entire revenue chain is untrustworthy.

use std::collections::BTreeMap;

use chrono::{DateTime, TimeZone, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InitDataError {
    #[error("telegram: initData missing hash")]
    MissingHash,
    #[error("telegram: initData signature mismatch")]
    BadSignature,
    #[error("telegram: initData expired")]
    Expired,
    #[error("telegram: initData missing user")]
    MissingUser,
    #[error("telegram: invalid auth_date")]
    BadAuthDate,
    #[error("telegram: failed to parse user")]
    BadUserJson,
}

/// Maximum accepted age for initData.
///
/// Telegram does not invalidate initData proactively, so freshness is entirely our
/// responsibility. Five minutes is enough for normal network and user delays while
/// keeping the replay window for intercepted initData very small. After verification,
/// issue our own short-lived JWT immediately; later requests must not replay initData.
pub const DEFAULT_MAX_AGE: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

/// User payload inside initData. Field names match Telegram's JSON.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub language_code: String,
    #[serde(default)]
    pub is_premium: bool,
}

/// Parsed initData after successful verification.
// auth_date / query_id have no consumers yet: expiry is checked in verify; query_id waits
// for features that call back to Telegram (send message, answer callback).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct InitData {
    pub user: User,
    pub auth_date: DateTime<Utc>,
    /// Value of `?startapp=`, i.e. tracking_id.
    pub start_param: Option<String>,
    pub query_id: Option<String>,
}

/// Verify initData signature and freshness; return parsed contents.
///
/// Algorithm (Telegram official definition):
///
/// 1. Extract and remove the `hash` field
/// 2. Sort remaining fields by key ascending into `k=v` lines joined by `\n` → data_check_string
/// 3. `secret = HMAC-SHA256(key="WebAppData", data=bot_token)`
/// 4. Expect `hash = HMAC-SHA256(key=secret, data=data_check_string)`
///
/// **Multi-tenant note**: each tenant uses its own bot; callers must resolve the tenant
/// and fetch the matching token first — never a single platform-wide token.
pub fn verify(
    raw_init_data: &str,
    bot_token: &str,
    max_age: chrono::TimeDelta,
    now: DateTime<Utc>,
) -> Result<InitData, InitDataError> {
    // BTreeMap is naturally key-sorted — exactly what data_check_string requires.
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    let mut given_hash = None;

    for (k, v) in parse_query(raw_init_data) {
        match k.as_str() {
            "hash" => given_hash = Some(v),
            // signature is Telegram's Ed25519 third-party check field; exclude from HMAC.
            // If not stripped, every real request carrying it would be wrongly rejected.
            "signature" => {}
            _ => {
                fields.insert(k, v);
            }
        }
    }

    let given_hash = given_hash.ok_or(InitDataError::MissingHash)?;

    let data_check_string = fields
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n");

    let secret = hmac_sha256(b"WebAppData", bot_token.as_bytes());
    let expected = hex::encode(hmac_sha256(&secret, data_check_string.as_bytes()));

    // Constant-time compare to avoid timing side-channels that leak the hash byte by byte.
    if !constant_time_eq(expected.as_bytes(), given_hash.as_bytes()) {
        return Err(InitDataError::BadSignature);
    }

    let auth_date_unix: i64 = fields
        .get("auth_date")
        .ok_or(InitDataError::BadAuthDate)?
        .parse()
        .map_err(|_| InitDataError::BadAuthDate)?;
    let auth_date = Utc
        .timestamp_opt(auth_date_unix, 0)
        .single()
        .ok_or(InitDataError::BadAuthDate)?;

    if max_age > chrono::TimeDelta::zero() && now - auth_date > max_age {
        return Err(InitDataError::Expired);
    }

    let user_json = fields.get("user").ok_or(InitDataError::MissingUser)?;
    let user: User = serde_json::from_str(user_json).map_err(|_| InitDataError::BadUserJson)?;
    if user.id == 0 {
        return Err(InitDataError::MissingUser);
    }

    Ok(InitData {
        user,
        auth_date,
        start_param: fields.get("start_param").cloned(),
        query_id: fields.get("query_id").cloned(),
    })
}

/// Read `start_param` (tracking_id) **before** signature verification.
///
/// This is the only field allowed pre-verify — it resolves the chicken-and-egg problem:
/// verify needs the bot token, the bot token needs the tenant, and the tenant is located
/// via this field.
///
/// Security-wise this is acceptable because it is used only for lookup, never authorisation:
/// forging it at worst resolves someone else's placement, then fails the initData signature
/// check that uses that tenant's bot token, which the attacker does not have.
pub fn start_param(raw_init_data: &str) -> Option<String> {
    parse_query(raw_init_data)
        .into_iter()
        .find(|(k, _)| k == "start_param")
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
}

fn parse_query(raw: &str) -> Vec<(String, String)> {
    raw.split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((
                urlencoding::decode(k).ok()?.into_owned(),
                urlencoding::decode(v).ok()?.into_owned(),
            ))
        })
        .collect()
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Constant-time equality. Do not use `==`: short-circuit comparison makes duration depend
/// on matching prefix length, letting an attacker brute-force the hash byte by byte.
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_BOT_TOKEN: &str = "123456:AAH-test-bot-token";

    /// Build valid initData using Telegram's official algorithm.
    fn sign_init_data(token: &str, fields: &[(&str, String)]) -> String {
        let sorted: BTreeMap<&str, &String> = fields.iter().map(|(k, v)| (*k, v)).collect();
        let dcs = sorted
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");

        let secret = hmac_sha256(b"WebAppData", token.as_bytes());
        let hash = hex::encode(hmac_sha256(&secret, dcs.as_bytes()));

        let mut parts: Vec<String> = fields
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect();
        parts.push(format!("hash={hash}"));
        parts.join("&")
    }

    fn valid_fields(auth_date: DateTime<Utc>) -> Vec<(&'static str, String)> {
        vec![
            ("auth_date", auth_date.timestamp().to_string()),
            ("query_id", "AAH123".into()),
            ("start_param", "aB3xY9zK1m".into()),
            (
                "user",
                r#"{"id":777001,"first_name":"Dave","username":"dave","is_premium":true}"#.into(),
            ),
        ]
    }

    #[test]
    fn accepts_valid_init_data() {
        let now = Utc::now();
        let raw = sign_init_data(TEST_BOT_TOKEN, &valid_fields(now));

        let got = verify(&raw, TEST_BOT_TOKEN, DEFAULT_MAX_AGE, now).expect("verification should pass");

        assert_eq!(got.user.id, 777_001);
        assert_eq!(got.user.username, "dave");
        assert!(got.user.is_premium);
        assert_eq!(
            got.start_param.as_deref(),
            Some("aB3xY9zK1m"),
            "start_param is tracking_id"
        );
    }

    #[test]
    fn rejects_wrong_token() {
        let now = Utc::now();
        let raw = sign_init_data(TEST_BOT_TOKEN, &valid_fields(now));

        let err = verify(&raw, "999999:another-tenant-token", DEFAULT_MAX_AGE, now).unwrap_err();

        assert_eq!(err, InitDataError::BadSignature);
    }

    /// Tampering any field must fail verification — otherwise an attacker could change
    /// start_param and redirect attribution to another KOL.
    #[test]
    fn rejects_tampered_field() {
        let now = Utc::now();
        let raw = sign_init_data(TEST_BOT_TOKEN, &valid_fields(now));
        let tampered = raw.replace("aB3xY9zK1m", "ATTACKER01");
        assert_ne!(raw, tampered);

        let err = verify(&tampered, TEST_BOT_TOKEN, DEFAULT_MAX_AGE, now).unwrap_err();

        assert_eq!(err, InitDataError::BadSignature);
    }

    /// Telegram does not invalidate initData; freshness is entirely our responsibility.
    #[test]
    fn rejects_expired() {
        let now = Utc::now();
        let raw = sign_init_data(
            TEST_BOT_TOKEN,
            &valid_fields(now - chrono::TimeDelta::minutes(30)),
        );

        let err = verify(&raw, TEST_BOT_TOKEN, DEFAULT_MAX_AGE, now).unwrap_err();

        assert_eq!(err, InitDataError::Expired);
    }

    #[test]
    fn rejects_missing_hash() {
        let err = verify(
            "auth_date=1&user=%7B%7D",
            TEST_BOT_TOKEN,
            DEFAULT_MAX_AGE,
            Utc::now(),
        )
        .unwrap_err();

        assert_eq!(err, InitDataError::MissingHash);
    }

    #[test]
    fn rejects_missing_user() {
        let now = Utc::now();
        let raw = sign_init_data(
            TEST_BOT_TOKEN,
            &[("auth_date", now.timestamp().to_string())],
        );

        let err = verify(&raw, TEST_BOT_TOKEN, DEFAULT_MAX_AGE, now).unwrap_err();

        assert_eq!(err, InitDataError::MissingUser);
    }

    /// signature is an Ed25519 third-party field; exclude from HMAC. If not stripped,
    /// every real request carrying it would be wrongly rejected.
    #[test]
    fn ignores_signature_field() {
        let now = Utc::now();
        let raw = sign_init_data(TEST_BOT_TOKEN, &valid_fields(now))
            + "&signature=ed25519-third-party-sig";

        verify(&raw, TEST_BOT_TOKEN, DEFAULT_MAX_AGE, now).expect("signature field must not affect verification");
    }
}
