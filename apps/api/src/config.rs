//! Runtime configuration.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub http: Http,
    pub postgres: Postgres,
    #[serde(default)]
    pub attribution: Attribution,
    #[serde(default)]
    pub secrets: Secrets,
}

/// Secret material.
///
/// **Neither key belongs in config files** — both are read from environment variables only.
/// Config files end up in version control or get copied around; if these keys leak, one
/// unlocks every tenant's bot token and API key, the other lets anyone forge player sessions.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Secrets {
    /// Master key for `_enc` fields, 32 bytes base64-encoded.
    /// Environment variable: `IGNITION_MASTER_KEY`
    #[serde(skip)]
    pub master_key_b64: String,
    /// Signing key for TMA session tokens.
    /// Environment variable: `IGNITION_JWT_KEY`
    #[serde(skip)]
    pub jwt_key: String,
    /// Envelope-encryption (KMS) settings. Absent ⇒ V1 direct master-key mode (the default).
    /// Present ⇒ new `_enc` writes are V2 envelope blobs; existing V1 blobs still decrypt.
    #[serde(default)]
    pub kms: Option<Kms>,
}

/// KMS envelope-encryption configuration.
///
/// This selects the [`KeyProvider`](crate::secrets::KeyProvider) that wraps per-secret data keys.
/// Only `local` is built into this binary today (credential-free, derives a KEK from the master
/// key — for local runs and CI). `aws` (and other real KMS backends) are deploy-time adapters:
/// implement the trait and register it in `main::build_cipher`.
#[derive(Debug, Clone, Deserialize)]
pub struct Kms {
    pub provider: KmsProvider,
    /// KMS key identifier (e.g. an AWS key ARN). Unused by `local`; the real KMS adapter (a
    /// deploy-time drop-in) will read it — hence no reader in this binary yet.
    #[serde(default)]
    #[allow(dead_code)]
    pub key_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KmsProvider {
    /// Local KEK derived from the master key — no external service.
    Local,
    /// AWS KMS — deploy-time adapter, not compiled into this binary.
    Aws,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Http {
    #[serde(default = "default_addr")]
    pub addr: String,
    /// Origins allowed to call `/v1/tma/*` cross-origin.
    ///
    /// The TMA frontend is a separately deployed static site on a different origin, so
    /// it must be explicitly allowed. **Use an allow-list, not `*`** — these endpoints carry
    /// Bearer tokens; a wildcard lets any page invoke our API with the user's token.
    /// Empty means CORS is disabled.
    #[serde(default)]
    pub cors_origins: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Postgres {
    pub dsn: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    /// Dedicated schema, isolated from other projects in a shared database.
    ///
    /// Same convention as growing-tales: every connection puts this first on `search_path`
    /// and all objects live here. When two projects share one Supabase instance, this layer
    /// is what prevents collisions — colliding table names are a data incident.
    #[serde(default = "default_schema")]
    pub schema: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Attribution {
    #[serde(default = "default_policy_version")]
    pub policy_version: String,
}

impl Default for Http {
    fn default() -> Self {
        Http {
            addr: default_addr(),
            cors_origins: Vec::new(),
        }
    }
}

impl Default for Attribution {
    fn default() -> Self {
        Attribution {
            policy_version: default_policy_version(),
        }
    }
}

fn default_addr() -> String {
    "0.0.0.0:8080".into()
}
fn default_max_connections() -> u32 {
    10
}
fn default_schema() -> String {
    "ignition".into()
}
fn default_policy_version() -> String {
    "v1".into()
}

impl Config {
    /// Load config file, with environment-variable overrides for sensitive values.
    ///
    /// DSN supports env override because config files are versioned and connection strings
    /// contain passwords.
    pub fn load(path: &str) -> anyhow::Result<Config> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config {path}: {e}"))?;
        let mut cfg: Config = serde_yaml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("failed to parse config {path}: {e}"))?;

        if let Ok(dsn) = std::env::var("IGNITION_PG_DSN") {
            cfg.postgres.dsn = dsn;
        }
        if let Ok(schema) = std::env::var("IGNITION_PG_SCHEMA") {
            cfg.postgres.schema = schema;
        }
        if !is_valid_ident(&cfg.postgres.schema) {
            anyhow::bail!("invalid postgres.schema: {}", cfg.postgres.schema);
        }

        // Keys come only from environment variables; missing means startup failure — no
        // "default if unset" fallback. A default signing key means every deployment shares one key.
        cfg.secrets.master_key_b64 = require_env("IGNITION_MASTER_KEY")?;
        cfg.secrets.jwt_key = require_env("IGNITION_JWT_KEY")?;

        Ok(cfg)
    }
}

/// Schema names are concatenated into SQL (`SET search_path` cannot be parameterised), so even
/// trusted config gets validated — never assume "this value is fine" at SQL splice sites.
fn is_valid_ident(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn require_env(name: &str) -> anyhow::Result<String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Ok(v),
        _ => Err(anyhow::anyhow!(
            "environment variable {name} is not set. Generate a new key: cargo run -- keygen"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Schema names are spliced into `SET search_path` (not parameterisable), so validation
    /// is a real injection defence, not ceremony.
    #[test]
    fn rejects_identifiers_that_could_inject_sql() {
        for bad in [
            "",
            "public; DROP SCHEMA ignition CASCADE",
            "igni tion",
            "ignition-app",
            "\"quoted\"",
            "1ignition",
            "igni'tion",
        ] {
            assert!(!is_valid_ident(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn accepts_normal_schema_names() {
        for ok in ["ignition", "growing_tales", "_tmp", "s1"] {
            assert!(is_valid_ident(ok), "{ok:?} should be accepted");
        }
    }

    #[test]
    fn rejects_over_length_identifiers() {
        // Postgres identifier limit is 63 bytes; longer names are silently truncated —
        // truncation can collide with another existing schema.
        assert!(is_valid_ident(&"a".repeat(63)));
        assert!(!is_valid_ident(&"a".repeat(64)));
    }
}
