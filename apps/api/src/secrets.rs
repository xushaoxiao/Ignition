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
//! - Ciphertext carries a version byte. V1 is local master-key encryption; V2 is KMS envelope
//!   encryption (per-secret data key wrapped by a [`KeyProvider`]). `open` dispatches on the
//!   version byte, so V1 blobs stay readable after a move to V2 — key rotation is inevitable,
//!   and formats without a version byte force downtime migrations.

use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Ciphertext format versions.
///
/// `V1` is direct master-key encryption: the master key comes from an environment variable,
/// suitable for local and single-node deployments. `V2` is envelope encryption — a fresh data
/// key (DEK) per secret, the DEK wrapped by a [`KeyProvider`] (a KMS in production). `open`
/// dispatches on the version byte, and V1 ciphertext remains readable regardless of the mode
/// the writer is in, so migrating to KMS needs no re-encryption downtime.
const V1: u8 = 1;
const V2: u8 = 2;

const NONCE_LEN: usize = 12;
const DEK_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secrets: master key must be 32 bytes, base64-encoded")]
    BadMasterKey,
    #[error("secrets: malformed ciphertext")]
    Malformed,
    #[error("secrets: unknown ciphertext version {0}")]
    UnknownVersion(u8),
    /// A V2 (envelope) blob was opened by a Cipher with no key provider configured — the data
    /// key cannot be unwrapped. Fail closed rather than guess.
    #[error("secrets: ciphertext is envelope-encrypted but no key provider is configured")]
    KeyProviderRequired,
    /// The key provider (KMS) failed to wrap or unwrap a data key.
    #[error("secrets: key provider operation failed")]
    KeyProvider,
    /// Decryption failure does not distinguish wrong key from tampered ciphertext — same
    /// handling for callers, and distinguishing would give attackers another observable signal.
    #[error("secrets: decryption failed")]
    Decrypt,
}

/// Wraps and unwraps per-secret data keys (DEKs).
///
/// This is the seam a real KMS plugs into: `wrap` is `kms:Encrypt`, `unwrap` is `kms:Decrypt`,
/// and the wrapped bytes are the KMS ciphertext blob. The master key never leaves the KMS.
/// [`LocalKeyProvider`] is the credential-free stand-in used locally and in tests; an AWS/GCP/Vault
/// adapter is a deploy-time drop-in — implement this trait and hand it to [`Cipher::with_kms`].
pub trait KeyProvider: Send + Sync {
    /// Wrap (encrypt) a freshly generated data key; the returned bytes are stored in the blob.
    fn wrap(&self, dek: &[u8]) -> Result<Vec<u8>, SecretError>;
    /// Unwrap (decrypt) a stored wrapped data key.
    fn unwrap(&self, wrapped: &[u8]) -> Result<Secret, SecretError>;
}

/// Envelope key provider backed by a local KEK — no external service.
///
/// The KEK is derived from the master key by domain-separated hashing, so enabling V2 locally
/// needs no extra key material. This gives the envelope *structure* (per-secret DEKs, a
/// version-tagged format, exercised wrap/unwrap paths) but not the isolation of a real KMS: the
/// root of trust is still the local master key. Use a KMS-backed provider in production; this one
/// is for local runs, CI, and validating the format end to end.
pub struct LocalKeyProvider {
    kek: Aes256Gcm,
}

impl LocalKeyProvider {
    /// Derive a KEK from the base64 master key.
    pub fn from_master_key_b64(b64: &str) -> Result<Self, SecretError> {
        let master = decode_key_32(b64)?;
        // Domain separation: the KEK must not equal the V1 data key even though both derive from
        // the same master, so a V1 and a V2 blob of the same plaintext share no key material.
        let mut hasher = Sha256::new();
        hasher.update(b"ignition:kms:local-kek:v2");
        hasher.update(master);
        let kek = hasher.finalize();
        Ok(LocalKeyProvider {
            kek: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&kek)),
        })
    }
}

impl KeyProvider for LocalKeyProvider {
    fn wrap(&self, dek: &[u8]) -> Result<Vec<u8>, SecretError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let ct = self
            .kek
            .encrypt(Nonce::from_slice(&nonce_bytes), dek)
            .map_err(|_| SecretError::KeyProvider)?;
        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    fn unwrap(&self, wrapped: &[u8]) -> Result<Secret, SecretError> {
        if wrapped.len() <= NONCE_LEN {
            return Err(SecretError::KeyProvider);
        }
        let (nonce, ct) = wrapped.split_at(NONCE_LEN);
        let dek = self
            .kek
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|_| SecretError::KeyProvider)?;
        Ok(Secret(dek))
    }
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

/// Encrypts and decrypts `_enc` columns.
///
/// Always holds the V1 master key so historical V1 blobs stay readable. When a `provider` is
/// present the Cipher **writes** V2 (envelope) blobs; reads dispatch on the version byte either way.
#[derive(Clone)]
pub struct Cipher {
    /// V1 data key — used to write V1 blobs (provider absent) and to read any V1 blob.
    v1: Aes256Gcm,
    /// Present ⇒ write V2 and unwrap V2 data keys. Absent ⇒ V1-only.
    provider: Option<Arc<dyn KeyProvider>>,
}

impl std::fmt::Debug for Cipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mode = if self.provider.is_some() { "v2" } else { "v1" };
        write!(f, "Cipher(<master key redacted>, mode={mode})")
    }
}

impl Cipher {
    /// Construct V1-mode: direct master-key encryption, no envelope provider.
    pub fn from_master_key_b64(b64: &str) -> Result<Self, SecretError> {
        let raw = decode_key_32(b64)?;
        Ok(Cipher {
            v1: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&raw)),
            provider: None,
        })
    }

    /// Construct V2-mode: seal produces envelope blobs via `provider`, while the master key is
    /// retained so any pre-existing V1 blob still decrypts. Migrating to KMS is thus write-forward
    /// with no re-encryption pass.
    pub fn with_kms(b64: &str, provider: Arc<dyn KeyProvider>) -> Result<Self, SecretError> {
        let raw = decode_key_32(b64)?;
        Ok(Cipher {
            v1: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&raw)),
            provider: Some(provider),
        })
    }

    /// Encrypt. V1 format: `[1][nonce 12B][ct+tag]`. V2 format:
    /// `[2][wrapped_len 2B BE][wrapped DEK][nonce 12B][ct+tag]`.
    ///
    /// Every call — and every wrapped DEK — uses a fresh random nonce. Reusing a nonce under GCM
    /// with the same key leaks plaintext XOR and breaks authentication; never derive nonces from
    /// counters or timestamps that might repeat.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>, SecretError> {
        match &self.provider {
            None => Ok(seal_gcm(&self.v1, V1, plaintext)),
            Some(provider) => self.seal_v2(provider.as_ref(), plaintext),
        }
    }

    fn seal_v2(
        &self,
        provider: &dyn KeyProvider,
        plaintext: &[u8],
    ) -> Result<Vec<u8>, SecretError> {
        // Fresh per-secret data key; wrapped by the provider, never stored in the clear.
        let mut dek = [0u8; DEK_LEN];
        rand::thread_rng().fill_bytes(&mut dek);
        let data = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&dek));
        let wrapped = provider.wrap(&dek);
        // Best-effort wipe of the plaintext DEK from our stack copy. A hardening pass would use the
        // `zeroize` crate for a volatile write the optimiser cannot elide.
        dek.fill(0);
        let wrapped = wrapped?;
        if wrapped.len() > u16::MAX as usize {
            return Err(SecretError::KeyProvider);
        }

        let inner = seal_gcm(&data, V2, plaintext);
        // `inner` is `[2][nonce][ct]`; splice the wrapped DEK in after the version byte.
        let (&version, body) = inner
            .split_first()
            .expect("seal_gcm always writes a version byte");
        let mut out = Vec::with_capacity(1 + 2 + wrapped.len() + body.len());
        out.push(version);
        out.extend_from_slice(&(wrapped.len() as u16).to_be_bytes());
        out.extend_from_slice(&wrapped);
        out.extend_from_slice(body);
        Ok(out)
    }

    /// Decrypt output from [`Cipher::seal`], dispatching on the version byte.
    pub fn open(&self, blob: &[u8]) -> Result<Secret, SecretError> {
        let (&version, rest) = blob.split_first().ok_or(SecretError::Malformed)?;
        match version {
            V1 => open_gcm(&self.v1, rest),
            V2 => {
                let provider = self
                    .provider
                    .as_ref()
                    .ok_or(SecretError::KeyProviderRequired)?;
                let (len_bytes, rest) = split_at_checked(rest, 2)?;
                let wrapped_len = u16::from_be_bytes([len_bytes[0], len_bytes[1]]) as usize;
                let (wrapped, rest) = split_at_checked(rest, wrapped_len)?;
                let dek = provider.unwrap(wrapped)?;
                let data =
                    Aes256Gcm::new_from_slice(dek.expose()).map_err(|_| SecretError::Decrypt)?;
                open_gcm(&data, rest)
            }
            other => Err(SecretError::UnknownVersion(other)),
        }
    }
}

/// GCM-seal with a fresh nonce; output is `[version][nonce 12B][ct+tag]`.
fn seal_gcm(cipher: &Aes256Gcm, version: u8, plaintext: &[u8]) -> Vec<u8> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    // AES-GCM encryption cannot fail when key and nonce are valid; this expect is unreachable.
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .expect("AES-GCM encryption should not fail");
    let mut out = Vec::with_capacity(1 + NONCE_LEN + ct.len());
    out.push(version);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    out
}

/// Decrypt a `[nonce 12B][ct+tag]` body (version byte already stripped).
fn open_gcm(cipher: &Aes256Gcm, body: &[u8]) -> Result<Secret, SecretError> {
    if body.len() <= NONCE_LEN {
        return Err(SecretError::Malformed);
    }
    let (nonce, ct) = body.split_at(NONCE_LEN);
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| SecretError::Decrypt)?;
    Ok(Secret(plain))
}

/// Split `s` at `mid`, returning `Malformed` instead of panicking when `s` is too short.
fn split_at_checked(s: &[u8], mid: usize) -> Result<(&[u8], &[u8]), SecretError> {
    if s.len() < mid {
        return Err(SecretError::Malformed);
    }
    Ok(s.split_at(mid))
}

/// Decode a base64 32-byte key.
fn decode_key_32(b64: &str) -> Result<Vec<u8>, SecretError> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| SecretError::BadMasterKey)?;
    if raw.len() != 32 {
        return Err(SecretError::BadMasterKey);
    }
    Ok(raw)
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

    /// A V2 (envelope) Cipher over the same master key, using the local key provider.
    fn kms_cipher(master_b64: &str) -> Cipher {
        let provider = LocalKeyProvider::from_master_key_b64(master_b64).unwrap();
        Cipher::with_kms(master_b64, Arc::new(provider)).unwrap()
    }

    #[test]
    fn round_trips() {
        let c = cipher();
        let blob = c.seal(b"123456:AA-bot-token").unwrap();
        assert_eq!(c.open(&blob).unwrap().expose(), b"123456:AA-bot-token");
    }

    /// Nonce must differ on every seal: reusing a nonce under GCM destroys confidentiality and integrity.
    #[test]
    fn each_seal_uses_a_fresh_nonce() {
        let c = cipher();
        let a = c.seal(b"same plaintext").unwrap();
        let b = c.seal(b"same plaintext").unwrap();
        assert_ne!(
            a, b,
            "two encryptions produced identical ciphertext — nonce was reused"
        );
    }

    #[test]
    fn rejects_tampered_ciphertext() {
        let c = cipher();
        let mut blob = c.seal(b"postback-secret").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        assert!(matches!(c.open(&blob), Err(SecretError::Decrypt)));
    }

    #[test]
    fn rejects_wrong_master_key() {
        let blob = cipher().seal(b"postback-secret").unwrap();
        assert!(matches!(cipher().open(&blob), Err(SecretError::Decrypt)));
    }

    #[test]
    fn rejects_unknown_version() {
        let c = cipher();
        let mut blob = c.seal(b"x").unwrap();
        blob[0] = 9;
        assert!(matches!(c.open(&blob), Err(SecretError::UnknownVersion(9))));
    }

    #[test]
    fn rejects_malformed_blob() {
        let c = cipher();
        assert!(matches!(c.open(&[]), Err(SecretError::Malformed)));
        assert!(matches!(c.open(&[V1, 0, 0]), Err(SecretError::Malformed)));
    }

    // ---- V2 envelope encryption ----

    #[test]
    fn v2_round_trips() {
        let master = generate_master_key_b64();
        let c = kms_cipher(&master);
        let blob = c.seal(b"123456:AA-bot-token").unwrap();
        assert_eq!(blob[0], V2, "V2 cipher must write a V2 blob");
        assert_eq!(c.open(&blob).unwrap().expose(), b"123456:AA-bot-token");
    }

    /// The whole point of the version byte: a Cipher migrated to V2 must still read old V1 blobs
    /// without a re-encryption pass.
    #[test]
    fn v2_cipher_still_reads_v1_blobs() {
        let master = generate_master_key_b64();
        let v1_blob = Cipher::from_master_key_b64(&master)
            .unwrap()
            .seal(b"legacy")
            .unwrap();
        assert_eq!(v1_blob[0], V1);

        let migrated = kms_cipher(&master);
        assert_eq!(migrated.open(&v1_blob).unwrap().expose(), b"legacy");
    }

    /// Opening a V2 blob with a V1-only Cipher must fail closed, not silently return garbage.
    #[test]
    fn v1_cipher_cannot_open_v2_blobs() {
        let master = generate_master_key_b64();
        let v2_blob = kms_cipher(&master).seal(b"x").unwrap();
        let v1_only = Cipher::from_master_key_b64(&master).unwrap();
        assert!(matches!(
            v1_only.open(&v2_blob),
            Err(SecretError::KeyProviderRequired)
        ));
    }

    #[test]
    fn v2_tampered_wrapped_dek_fails() {
        let master = generate_master_key_b64();
        let c = kms_cipher(&master);
        let mut blob = c.seal(b"secret").unwrap();
        // Flip a byte inside the wrapped DEK region ([1]=version, [2..4]=len, wrapped starts at 3).
        blob[4] ^= 0x01;
        assert!(c.open(&blob).is_err());
    }

    #[test]
    fn v2_each_seal_uses_fresh_dek_and_nonce() {
        let master = generate_master_key_b64();
        let c = kms_cipher(&master);
        let a = c.seal(b"same").unwrap();
        let b = c.seal(b"same").unwrap();
        assert_ne!(
            a, b,
            "V2 blobs of the same plaintext must differ (fresh DEK + nonce)"
        );
    }

    /// A different master key derives a different local KEK, so its provider cannot unwrap the DEK.
    #[test]
    fn v2_wrong_master_key_cannot_unwrap() {
        let blob = kms_cipher(&generate_master_key_b64())
            .seal(b"secret")
            .unwrap();
        let other = kms_cipher(&generate_master_key_b64());
        assert!(other.open(&blob).is_err());
    }

    #[test]
    fn v2_rejects_truncated_blob() {
        let master = generate_master_key_b64();
        let c = kms_cipher(&master);
        // Version byte present but the wrapped-length header is missing.
        assert!(matches!(c.open(&[V2]), Err(SecretError::Malformed)));
        // Length header claims more wrapped bytes than remain.
        assert!(matches!(
            c.open(&[V2, 0xff, 0xff, 0x00]),
            Err(SecretError::Malformed)
        ));
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
