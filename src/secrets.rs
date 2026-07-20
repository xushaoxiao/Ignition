//! 密钥的加密存储与防泄漏封装。
//!
//! 表结构里所有 `_enc` 后缀的字段 —— `bot.token_enc` 与
//! `api_key.secret_enc` —— 存的都是本模块产出的密文。
//!
//! 有两条约束交给类型承担，而不是交给「大家记得」：
//!
//! - 解密结果包在 [`Secret`] 里，它的 `Debug` 只打印 `<redacted>`。密钥进日志
//!   最常见的路径不是有人手写 `println!(token)`，而是某个 `#[derive(Debug)]` 的
//!   结构体里恰好有一个密钥字段，然后被 `tracing::info!(?req)` 整个打了出来。
//!   把密钥装进一个 Debug 已被劫持的容器，这条路径就断了。
//! - 密文带版本字节。现在是本地主密钥直接加密，将来换成 KMS 信封加密时
//!   会是新版本号，而旧密文仍然解得开 —— 密钥轮换是必然会发生的事，
//!   格式里不留版本位，到时候就只能停机迁移。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use rand::RngCore;

/// 密文格式的版本。
///
/// `V1` 是主密钥直接加密：主密钥来自环境变量，适合本地与单机部署。
/// 将来的 `V2` 会是 KMS 信封加密（每条密钥一个 DEK，DEK 由 KMS 主密钥包裹），
/// 届时 `open` 按版本字节分派，V1 密文继续可读。
const V1: u8 = 1;

const NONCE_LEN: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secrets: 主密钥必须是 base64 编码的 32 字节")]
    BadMasterKey,
    #[error("secrets: 密文格式非法")]
    Malformed,
    #[error("secrets: 未知的密文版本 {0}")]
    UnknownVersion(u8),
    /// 解密失败不区分「密钥不对」与「密文被篡改」—— 对调用方而言处置相同，
    /// 而区分开来只会给攻击者多一个可观测的信号。
    #[error("secrets: 解密失败")]
    Decrypt,
}

/// 一段解密后的密钥材料。
///
/// 刻意不实现 `Display`、不派生 `Serialize`：想把它写出去必须显式调
/// [`Secret::expose`]，那是一处可以被 review 抓住的调用，而不是一次无意的格式化。
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Vec<u8>);

impl Secret {
    /// 目前只有测试与未来的 KMS 实现会用到 —— 在线链路里 `Secret` 一律由
    /// [`Cipher::open`] 产出。
    #[allow(dead_code)]
    pub fn new(bytes: Vec<u8>) -> Self {
        Secret(bytes)
    }

    /// 取出明文字节。调用点应当尽可能少，且不应把返回值再放进任何会被日志
    /// 打印的结构体里。
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// 以 UTF-8 字符串取出明文。Bot token 与 postback secret 都是文本形态。
    pub fn expose_str(&self) -> Result<&str, SecretError> {
        std::str::from_utf8(&self.0).map_err(|_| SecretError::Malformed)
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// 主密钥持有者，负责 `_enc` 字段的加解密。
#[derive(Clone)]
pub struct Cipher {
    inner: Aes256Gcm,
}

impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cipher(<master key redacted>)")
    }
}

impl Cipher {
    /// 从 base64 编码的 32 字节主密钥构造。
    pub fn from_master_key_b64(b64: &str) -> Result<Self, SecretError> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|_| SecretError::BadMasterKey)?;
        if raw.len() != 32 {
            return Err(SecretError::BadMasterKey);
        }
        let key = Key::<Aes256Gcm>::from_slice(&raw);
        Ok(Cipher {
            inner: Aes256Gcm::new(key),
        })
    }

    /// 加密。输出格式：`[版本 1B][nonce 12B][密文 + 认证标签]`。
    ///
    /// 每次调用生成新的随机 nonce。GCM 在同一密钥下重用 nonce 会直接泄漏明文
    /// 异或值并使认证失效，所以 nonce 绝不能取计数器或时间戳这类可能重放的来源。
    pub fn seal(&self, plaintext: &[u8]) -> Vec<u8> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // AES-GCM 的加密在密钥与 nonce 合法时不会失败，这里的 expect 不可达。
        let ct = self
            .inner
            .encrypt(nonce, plaintext)
            .expect("AES-GCM 加密不应失败");

        let mut out = Vec::with_capacity(1 + NONCE_LEN + ct.len());
        out.push(V1);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        out
    }

    /// 解密 [`Cipher::seal`] 的输出。
    pub fn open(&self, blob: &[u8]) -> Result<Secret, SecretError> {
        let (&version, rest) = blob.split_first().ok_or(SecretError::Malformed)?;
        if version != V1 {
            return Err(SecretError::UnknownVersion(version));
        }
        if rest.len() <= NONCE_LEN {
            return Err(SecretError::Malformed);
        }
        let (nonce_bytes, ct) = rest.split_at(NONCE_LEN);

        let plain = self
            .inner
            .decrypt(Nonce::from_slice(nonce_bytes), ct)
            .map_err(|_| SecretError::Decrypt)?;
        Ok(Secret(plain))
    }
}

/// 生成一把新的主密钥，base64 编码。供 `ignition keygen` 使用。
pub fn generate_master_key_b64() -> String {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    base64::engine::general_purpose::STANDARD.encode(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> Cipher {
        Cipher::from_master_key_b64(&generate_master_key_b64()).unwrap()
    }

    #[test]
    fn round_trips() {
        let c = cipher();
        let blob = c.seal(b"123456:AA-bot-token");
        assert_eq!(c.open(&blob).unwrap().expose(), b"123456:AA-bot-token");
    }

    /// nonce 必须每次不同：GCM 下重用 nonce 会同时毁掉机密性和完整性。
    #[test]
    fn each_seal_uses_a_fresh_nonce() {
        let c = cipher();
        let a = c.seal(b"same plaintext");
        let b = c.seal(b"same plaintext");
        assert_ne!(a, b, "两次加密产出了相同密文，nonce 被重用了");
    }

    #[test]
    fn rejects_tampered_ciphertext() {
        let c = cipher();
        let mut blob = c.seal(b"postback-secret");
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        assert!(matches!(c.open(&blob), Err(SecretError::Decrypt)));
    }

    #[test]
    fn rejects_wrong_master_key() {
        let blob = cipher().seal(b"postback-secret");
        assert!(matches!(cipher().open(&blob), Err(SecretError::Decrypt)));
    }

    #[test]
    fn rejects_unknown_version() {
        let c = cipher();
        let mut blob = c.seal(b"x");
        blob[0] = 9;
        assert!(matches!(c.open(&blob), Err(SecretError::UnknownVersion(9))));
    }

    #[test]
    fn rejects_malformed_blob() {
        let c = cipher();
        assert!(matches!(c.open(&[]), Err(SecretError::Malformed)));
        assert!(matches!(c.open(&[V1, 0, 0]), Err(SecretError::Malformed)));
    }

    #[test]
    fn master_key_must_be_32_bytes() {
        assert!(Cipher::from_master_key_b64("c2hvcnQ=").is_err());
        assert!(Cipher::from_master_key_b64("!!!not base64!!!").is_err());
        assert!(Cipher::from_master_key_b64(&generate_master_key_b64()).is_ok());
    }

    /// 这条测试守护硬性规则「Bot token 与 postback secret 禁止进日志」：
    /// 密钥最常见的泄漏路径是被 derive(Debug) 的结构体连带打出来。
    #[test]
    fn secret_never_leaks_through_debug() {
        #[derive(Debug)]
        struct Holder {
            #[allow(dead_code)]
            token: Secret,
        }
        let h = Holder {
            token: Secret::new(b"123456:AA-super-secret".to_vec()),
        };
        let rendered = format!("{h:?}");
        assert!(
            !rendered.contains("super-secret"),
            "密钥出现在 Debug 输出里"
        );
        assert!(rendered.contains("redacted"));
    }
}
