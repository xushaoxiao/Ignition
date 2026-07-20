//! TMA 会话令牌。
//!
//! 终端用户走 Telegram Mini App 进来，身份来源是 initData。initData 只在打开
//! 那一刻有效（我们自己把时效卡在 5 分钟，见 `telegram::DEFAULT_MAX_AGE`），
//! 之后的每个请求不能再重放它 —— 重放意味着一份被截获的 initData 可以长期
//! 冒充该用户。所以校验通过后立刻换发我们自己签发的短期令牌。
//!
//! 两种令牌：
//!
//! - **access**：15 分钟，带在每个业务请求上。
//! - **refresh**：7 天，只能用来换新的 access。
//!
//! 为什么需要 refresh：initData 在页面生命周期内不会更新，而用户可能把 Mini App
//! 挂在后台超过 15 分钟。没有 refresh，用户回来时要么被登出、要么我们被迫把
//! access 的时效放长到几小时 —— 后者等于放大了令牌泄漏的影响面。
//!
//! **已知取舍**：refresh 是无状态 JWT，签发后在到期前无法单独吊销，只能靠轮换
//! 签名密钥整体失效。TMA 会话的权限很低（只能玩自己的游戏、领自己的码），
//! 这个代价可以接受；等 KOL 后台上线，那一侧的会话必须改成有状态的。

use chrono::{DateTime, TimeDelta, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

pub const ACCESS_TTL: TimeDelta = TimeDelta::minutes(15);
pub const REFRESH_TTL: TimeDelta = TimeDelta::days(7);

/// 令牌用途。
///
/// 必须进 claims 并在校验时比对：否则一个 7 天的 refresh 令牌可以直接当作
/// access 用，等于把 access 的时效偷偷变成了 7 天。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    Access,
    Refresh,
}

/// 会话内容。
///
/// 归因所需的四个 ID（campaign / link / kol / player）在换发令牌时就固定下来，
/// 业务请求不再从客户端读取它们 —— 否则前端可以自己改 `kol_id`，把转化记到
/// 任意 KOL 名下，归因数据也就没有可信度可言了。
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
    #[error("jwt: 令牌无效或已过期")]
    Invalid,
    #[error("jwt: 令牌用途不符")]
    WrongKind,
}

/// 令牌签发与校验。
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

/// 换发结果。
#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    /// access 的剩余秒数，让前端可以在到期前主动刷新而不是等 401。
    pub expires_in: i64,
}

impl Issuer {
    pub fn new(signing_key: &[u8]) -> Self {
        Issuer {
            encoding: EncodingKey::from_secret(signing_key),
            decoding: DecodingKey::from_secret(signing_key),
        }
    }

    /// 签发一对令牌。
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
        // HS256 + 固定 claims 结构下编码不会失败。
        jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .expect("JWT 编码不应失败")
    }

    /// 校验令牌并确认用途。
    ///
    /// `now` 由调用方注入而不是读真实时钟，测试才能不 sleep 地覆盖过期分支。
    pub fn verify(
        &self,
        token: &str,
        want: TokenKind,
        now: DateTime<Utc>,
    ) -> Result<Claims, JwtError> {
        let mut validation = Validation::new(Algorithm::HS256);
        // 自己按注入的 now 判过期，所以关掉库内基于系统时钟的校验。
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

/// 签发令牌所需的会话主体。
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

    /// refresh 令牌不能当 access 用，否则 access 的 15 分钟时效形同虚设。
    #[test]
    fn refresh_token_is_not_accepted_as_access() {
        let iss = issuer();
        let s = iss.issue(&subject(), now());

        let err = iss
            .verify(&s.refresh_token, TokenKind::Access, now())
            .unwrap_err();
        assert_eq!(err, JwtError::WrongKind);

        iss.verify(&s.refresh_token, TokenKind::Refresh, now())
            .expect("用途相符时应通过");
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
        .expect("未到期应通过");

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

        assert!(iss
            .verify(&s.access_token, TokenKind::Access, later)
            .is_err());
        assert!(iss
            .verify(&s.refresh_token, TokenKind::Refresh, later)
            .is_ok());
    }

    /// 换一把签名密钥就应当全部失效 —— 这是无状态 refresh 唯一的吊销手段。
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

        // 改动 payload 段的一个字符，签名随即对不上。
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

    /// 签名密钥被劫持的 `alg: none` 攻击：库侧固定 HS256，这里再守一道。
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
