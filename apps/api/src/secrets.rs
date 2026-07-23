//! Encrypted secret storage and leak-prevention wrappers.
//!
//! Every `_enc` column in the schema — `bot.token_enc` and `api_key.secret_enc` —
//! stores ciphertext produced by this module.
//!
//! Two constraints are enforced by types rather than by convention:
//!
//! - Decrypted material is wrapped in [`Secret`], whose `Debug` always prints
//!   `<redacted>`. The most common leak path is not someone calling `println!(token)`,
//!   but a `#[derive(Debug)]` struct that happens to contain a key field and gets
//!   logged wholesale via `tracing::info!(?req)`. Putting secrets in a container with
//!   a hijacked `Debug` breaks that path.
//! - Ciphertext carries a version byte. V1 is local master-key encryption today; a
//!   future V2 will be KMS envelope encryption (per-secret DEK wrapped by a KMS master key).
//!   `open` will dispatch on version so V1 blobs remain readable — key rotation is
//!   inevitable, and formats without a version byte force downtime migrations.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use rand::RngCore;

/// Ciphertext format version.
///
/// `V1` is direct master-key encryption: the master key comes from an environment variable,
/// suitable for local and single-node deployments. A future `V2` will be KMS envelope
/// encryption (one DEK per secret, DEK wrapped by the KMS master key); `open` will dispatch
/// on the version byte and V1 ciphertext will remain readable.
const V1: u8 = 1;

const NONCE_LEN: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secrets: master key must be 32 bytes, base64-encoded")]
    BadMasterKey,
    #[error("secrets: malformed ciphertext")]
    Malformed,
    #[error("secrets: unknown ciphertext version {0}")]
    UnknownVersion(u8),
    /// Decryption failure does not distinguish wrong key from tampered ciphertext — same
    /// handling for callers, and distinguishing would give attackers another observable signal.
    #[error("secrets: decryption failed")]
    Decrypt,
}

/// A slice of decrypted key material.
///
/// Deliberately no `Display`, no `Serialize`: writing it out requires an explicit
/// [`Secret::expose`] call — a reviewable boundary, not accidental formatting.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Vec<u8>);

impl Secret {
    /// Used only by tests and a future KMS implementation — online paths always produce
    /// `Secret` via [`Cipher::open`].
    #[allow(dead_code)]
    pub fn new(bytes: Vec<u8>) -> Self {
        Secret(bytes)
    }

    /// Expose plaintext bytes. Call sites should be minimal; do not put the return value
    /// into any struct that might be logged.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Expose plaintext as a UTF-8 string. Bot tokens and postback secrets are textual.
    pub fn expose_str(&self) -> Result<&str, SecretError> {
        std::str::from_utf8(&self.0).map_err(|_| SecretError::Malformed)
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Holds the master key and encrypts/decrypts `_enc` columns.
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
    /// Construct from a base64-encoded 32-byte master key.
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

    /// Encrypt. Output format: `[version 1B][nonce 12B][ciphertext + auth tag]`.
    ///
    /// Each call generates a fresh random nonce. Reusing a nonce under GCM with the same key
    /// leaks plaintext XOR and breaks authentication — never use counters or timestamps that
    /// might repeat.
    pub fn seal(&self, plaintext: &[u8]) -> Vec<u8> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        // AES-GCM encryption cannot fail when key and nonce are valid; this expect is unreachable.
        let ct = self
            .inner
            .encrypt(nonce, plaintext)
            .expect("AES-GCM encryption should not fail");

        let mut out = Vec::with_capacity(1 + NONCE_LEN + ct.len());
        out.push(V1);
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        out
    }

    /// Decrypt output from [`Cipher::seal`].
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

/// Generate a new master key, base64-encoded. Used by `ignition keygen`.
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

    /// Nonce must differ on every seal: reusing a nonce under GCM destroys confidentiality and integrity.
    #[test]
    fn each_seal_uses_a_fresh_nonce() {
        let c = cipher();
        let a = c.seal(b"same plaintext");
        let b = c.seal(b"same plaintext");
        assert_ne!(
            a, b,
            "two encryptions produced identical ciphertext — nonce was reused"
        );
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

    /// Guards the hard rule that bot tokens and postback secrets must never reach logs:
    /// the most common leak is a derive(Debug) struct logging them incidentally.
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
            "secret material appeared in Debug output"
        );
        assert!(rendered.contains("redacted"));
    }
}
