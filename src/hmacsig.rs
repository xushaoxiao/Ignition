//! 主 App 回传（S2S postback）的签名与校验。
//!
//! 回传接口是唯一由客户主动调用、且直接产生可计费事件的入口，必须同时防伪造
//! 和防重放。

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::telegram::constant_time_eq;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SigError {
    #[error("hmacsig: 签名不匹配")]
    BadSignature,
    #[error("hmacsig: 时间戳超出允许窗口")]
    Stale,
    #[error("hmacsig: 时间戳格式非法")]
    BadTimestamp,
}

/// 允许的时钟偏移窗口。
///
/// 双向 5 分钟：客户服务器的时钟未必与我们同步，窗口太窄会造成大量误拒
/// （表现为客户「回传丢失」，直接影响收入统计）；太宽则放大重放窗口。
pub const DEFAULT_SKEW: TimeDelta = TimeDelta::minutes(5);

/// 计算签名：`HMAC-SHA256(secret, timestamp + "." + body)`。
///
/// 时间戳纳入签名范围，使其无法被篡改 —— 否则攻击者可以拿一个旧请求改时间戳
/// 后无限重放。
pub fn sign(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC 接受任意长度密钥");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// 校验签名与时间戳窗口。
///
/// 这里只挡住重放的「时间窗」这一半，另一半靠 `billable_event` 上
/// `(tenant_id, event_type, external_id)` 的唯一约束 —— 窗口内的重放会被
/// 幂等吃掉，不会产生第二笔计费。两者缺一不可。
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
        .expect("应校验通过");
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

    /// 时间戳纳入签名范围，改了时间戳签名就对不上 —— 攻击者无法拿旧请求
    /// 改时间戳后无限重放。
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

    /// 窗口是双向的：客户服务器时钟快于我们时同样要拒绝，否则攻击者可以
    /// 签一个未来的时间戳换取一个超长的有效期。
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
