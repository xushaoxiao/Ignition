//! API Key + HMAC 请求签名（S2S）。
//!
//! 这一模块替换掉了此前从 `X-Tenant-ID` 头读租户的占位实现。那个实现意味着
//! 任何人只要猜到一个租户 ID 和一个领奖码，就能核销别人的码、伪造别人的账单
//! —— 收入正确性的前提是「调用方确实是他声称的那个租户」。
//!
//! 请求头：
//!
//! ```text
//! X-Ignition-Key:       ik_live_xxx        密钥标识，明文
//! X-Ignition-Timestamp: 1753056000         Unix 秒
//! X-Ignition-Signature: <hex>              HMAC-SHA256
//! ```
//!
//! 签名对象是**规范化请求**而非仅请求体，见 [`canonical_request`]。

use chrono::{DateTime, TimeDelta, Utc};

use crate::hmacsig::{self, SigError};

pub const HEADER_KEY: &str = "X-Ignition-Key";
pub const HEADER_TIMESTAMP: &str = "X-Ignition-Timestamp";
pub const HEADER_SIGNATURE: &str = "X-Ignition-Signature";

/// 允许的时钟偏移，与回传接口取同一个值。
pub const DEFAULT_SKEW: TimeDelta = hmacsig::DEFAULT_SKEW;

/// API Key 的权限范围。
///
/// 用穷尽 `match` 解析而不是直接比字符串：新增一种能力时，
/// 编译器会强制在这里登记，不会出现「数据库里写了个 scope，代码里没人认识，
/// 于是静默失效」的情况。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// 核销领奖码
    Redeem,
    /// 变现回传
    Postback,
    /// 查询归因
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

/// 构造被签名的规范化请求：`METHOD\n路径\n请求体`。
///
/// **方法与路径必须进签名范围。** 否则一个对 `/v1/claims/redeem` 合法的签名，
/// 可以被原样重放到 `/v1/postback/purchase` 上 —— 请求体不同的两个接口之间
/// 本来不会互相冒充，但如果只签请求体，攻击者截获任一请求后就能挑选打到哪个
/// 接口。签名要绑定「这个请求」，不是「这段字节」。
pub fn canonical_request(method: &str, path: &str, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(method.len() + path.len() + body.len() + 2);
    out.extend_from_slice(method.to_ascii_uppercase().as_bytes());
    out.push(b'\n');
    out.extend_from_slice(path.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(body);
    out
}

/// 计算一个请求的签名。
///
/// 服务端本身只验签不签名，所以这个函数在在线链路上没有调用点 —— 它的用途是
/// 接入文档与客户端 SDK 的参考实现，以及测试里构造合法请求。放在这里而不是
/// 文档里，是为了让签名算法只有一处定义：文档写错了没人会发现，代码写错了
/// 上面的测试会红。
#[allow(dead_code)]
pub fn sign(secret: &[u8], timestamp: i64, method: &str, path: &str, body: &[u8]) -> String {
    hmacsig::sign(secret, timestamp, &canonical_request(method, path, body))
}

/// 校验请求签名与时效。
///
/// 时效窗口只挡住重放的一半；另一半靠写接口自身的幂等约束
/// （`billable_event` 的唯一键、`claim_code` 的状态机）。两者缺一不可：
/// 只有窗口，窗口内可重放；只有幂等，攻击者可以无限期重放拿到相同响应
/// 从而探测系统状态。
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

/// 检查密钥是否具备某项权限。
///
/// 权限缺省是「没有」：`scopes` 里没写就没有。给主 App 的密钥通常只需要
/// `redeem`，即便密钥泄漏，攻击者也伪造不了变现回传。
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
        verify_at("POST", "/v1/claims/redeem", BODY, &sig).expect("应校验通过");
    }

    /// 签名必须绑定路径，否则核销接口的签名能被重放到回传接口上 ——
    /// 那等于用一个只该核销的密钥伪造出可计费的变现事件。
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

    /// 权限缺省为「没有」：数据库里没写的 scope 不会被意外放行。
    #[test]
    fn scopes_default_to_denied() {
        let only_redeem = vec!["redeem".to_string()];
        assert!(has_scope(&only_redeem, Scope::Redeem));
        assert!(!has_scope(&only_redeem, Scope::Postback));
        assert!(!has_scope(&[], Scope::Redeem));
    }

    /// 无法识别的 scope 字符串不应放行任何权限 —— 拼错了要表现为「没权限」，
    /// 而不是碰巧匹配上别的。
    #[test]
    fn unknown_scope_strings_grant_nothing() {
        let typo = vec!["redeeem".to_string(), "*".to_string()];
        for want in [Scope::Redeem, Scope::Postback, Scope::ReadAttribution] {
            assert!(!has_scope(&typo, want), "{want:?} 不应被放行");
        }
    }

    #[test]
    fn scope_strings_round_trip() {
        for s in [Scope::Redeem, Scope::Postback, Scope::ReadAttribution] {
            assert_eq!(Scope::parse(s.as_str()), Some(s));
        }
    }
}
