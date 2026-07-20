//! Telegram Mini App 的 initData 校验。
//!
//! 这是系统的第一道信任边界：initData 决定了「这个请求来自哪个 TG 用户」，
//! 而该身份最终会通过领奖码核销绑定到可计费的归因上。校验一旦被绕过，
//! 整条收入链路都不可信。

use std::collections::BTreeMap;

use chrono::{DateTime, TimeZone, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InitDataError {
    #[error("telegram: initData 缺少 hash")]
    MissingHash,
    #[error("telegram: initData 签名不匹配")]
    BadSignature,
    #[error("telegram: initData 已过期")]
    Expired,
    #[error("telegram: initData 缺少 user")]
    MissingUser,
    #[error("telegram: auth_date 非法")]
    BadAuthDate,
    #[error("telegram: user 解析失败")]
    BadUserJson,
}

/// initData 的最大接受时效。
///
/// Telegram 不会主动使 initData 失效，所以时效完全由我们把关。取 5 分钟：
/// 足够覆盖正常的网络与用户操作延迟，又能把被截获的 initData 的可重放窗口
/// 压到很小。校验通过后应立即换发自己的短期 JWT，后续请求不再重放 initData。
pub const DEFAULT_MAX_AGE: chrono::TimeDelta = chrono::TimeDelta::minutes(5);

/// initData 里的用户信息。字段名对应 Telegram 的 JSON。
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

/// 校验通过后的 initData 内容。
#[derive(Debug, Clone)]
pub struct InitData {
    pub user: User,
    pub auth_date: DateTime<Utc>,
    /// `?startapp=` 的值，即 tracking_id
    pub start_param: Option<String>,
    pub query_id: Option<String>,
}

/// 校验 initData 的签名与时效，返回其内容。
///
/// 算法（Telegram 官方定义）：
///
/// 1. 取出并移除 `hash` 字段
/// 2. 其余字段按 key 升序排成 `k=v` 并用 `\n` 连接，得到 data_check_string
/// 3. `secret = HMAC-SHA256(key="WebAppData", data=bot_token)`
/// 4. 期望 `hash = HMAC-SHA256(key=secret, data=data_check_string)`
///
/// **多租户注意**：每个租户用自己的 Bot，调用方必须先定位租户、取对应 token，
/// 不能用平台级的单一 token 校验。
pub fn verify(
    raw_init_data: &str,
    bot_token: &str,
    max_age: chrono::TimeDelta,
    now: DateTime<Utc>,
) -> Result<InitData, InitDataError> {
    // BTreeMap 天然按 key 排序，正好是 data_check_string 要求的顺序。
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    let mut given_hash = None;

    for (k, v) in parse_query(raw_init_data) {
        match k.as_str() {
            "hash" => given_hash = Some(v),
            // signature 是 Telegram 的 Ed25519 第三方校验字段，不参与 HMAC
            // 计算。若未剔除，所有带该字段的真实请求都会被误拒。
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

    // 定长比较，避免通过响应时间侧信道逐字节爆破 hash。
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
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC 接受任意长度密钥");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// 定长比较。不用 `==`：短路比较会让比较耗时随匹配前缀长度变化，
/// 攻击者可以据此逐字节爆破出正确的 hash。
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

    /// 按 Telegram 官方算法构造一份合法的 initData。
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

        let got = verify(&raw, TEST_BOT_TOKEN, DEFAULT_MAX_AGE, now).expect("应校验通过");

        assert_eq!(got.user.id, 777_001);
        assert_eq!(got.user.username, "dave");
        assert!(got.user.is_premium);
        assert_eq!(
            got.start_param.as_deref(),
            Some("aB3xY9zK1m"),
            "start_param 即 tracking_id"
        );
    }

    #[test]
    fn rejects_wrong_token() {
        let now = Utc::now();
        let raw = sign_init_data(TEST_BOT_TOKEN, &valid_fields(now));

        let err = verify(&raw, "999999:another-tenant-token", DEFAULT_MAX_AGE, now).unwrap_err();

        assert_eq!(err, InitDataError::BadSignature);
    }

    /// 篡改任意字段都必须导致签名失败 —— 否则攻击者可以改 start_param
    /// 把归因转给别的 KOL。
    #[test]
    fn rejects_tampered_field() {
        let now = Utc::now();
        let raw = sign_init_data(TEST_BOT_TOKEN, &valid_fields(now));
        let tampered = raw.replace("aB3xY9zK1m", "ATTACKER01");
        assert_ne!(raw, tampered);

        let err = verify(&tampered, TEST_BOT_TOKEN, DEFAULT_MAX_AGE, now).unwrap_err();

        assert_eq!(err, InitDataError::BadSignature);
    }

    /// Telegram 不会主动使 initData 失效，时效完全由我们把关。
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

    /// signature 是 Ed25519 第三方校验字段，不参与 HMAC 计算。若未剔除，
    /// 所有带该字段的真实请求都会被误拒。
    #[test]
    fn ignores_signature_field() {
        let now = Utc::now();
        let raw = sign_init_data(TEST_BOT_TOKEN, &valid_fields(now))
            + "&signature=ed25519-third-party-sig";

        verify(&raw, TEST_BOT_TOKEN, DEFAULT_MAX_AGE, now).expect("signature 字段不应影响校验");
    }
}
