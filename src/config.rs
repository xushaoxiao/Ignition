//! 运行配置。

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

/// 密钥材料。
///
/// **两把都不进配置文件**，只从环境变量读 —— 配置文件会进版本库或被拷来拷去，
/// 而这两把钥匙一旦泄漏，前者能解开所有租户的 Bot token 与 API 密钥，
/// 后者能伪造任意玩家的会话。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Secrets {
    /// `_enc` 字段的主密钥，base64 编码的 32 字节。
    /// 环境变量：`IGNITION_MASTER_KEY`
    #[serde(skip)]
    pub master_key_b64: String,
    /// TMA 会话令牌的签名密钥。
    /// 环境变量：`IGNITION_JWT_KEY`
    #[serde(skip)]
    pub jwt_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Http {
    #[serde(default = "default_addr")]
    pub addr: String,
    /// 允许跨域访问 `/v1/tma/*` 的来源。
    ///
    /// TMA 前端是独立部署的静态站点，与 API 不同源，所以必须显式放行。
    /// **用允许列表而不是 `*`**：这些接口带着 Bearer 令牌，通配符等于允许
    /// 任意页面拿着用户的令牌调我们的接口。留空则不开启 CORS。
    #[serde(default)]
    pub cors_origins: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Postgres {
    pub dsn: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    /// 独立 schema，与共享库里的其它项目隔离。
    ///
    /// 与 growing-tales 同一套约定：每条连接把它放到 `search_path` 首位，
    /// 所有对象都落在这里。两个项目共用一个 Supabase 实例时，
    /// 靠的就是这一层隔离 —— 表名撞车的代价是数据事故。
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
    /// 读取配置文件，并允许用环境变量覆盖敏感项。
    ///
    /// DSN 之所以支持环境变量覆盖：配置文件会进版本库，而连接串含密码。
    pub fn load(path: &str) -> anyhow::Result<Config> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("读取配置 {path} 失败: {e}"))?;
        let mut cfg: Config =
            serde_yaml::from_str(&raw).map_err(|e| anyhow::anyhow!("解析配置 {path} 失败: {e}"))?;

        if let Ok(dsn) = std::env::var("IGNITION_PG_DSN") {
            cfg.postgres.dsn = dsn;
        }
        if let Ok(schema) = std::env::var("IGNITION_PG_SCHEMA") {
            cfg.postgres.schema = schema;
        }
        if !is_valid_ident(&cfg.postgres.schema) {
            anyhow::bail!("非法的 postgres.schema: {}", cfg.postgres.schema);
        }

        // 密钥只从环境变量读，缺失即启动失败 —— 不提供「没配就用默认值」的
        // 兜底。一个有默认值的签名密钥，等于所有部署共用同一把钥匙。
        cfg.secrets.master_key_b64 = require_env("IGNITION_MASTER_KEY")?;
        cfg.secrets.jwt_key = require_env("IGNITION_JWT_KEY")?;

        Ok(cfg)
    }
}

/// schema 名会被拼进 SQL（`SET search_path` 不接受参数绑定），所以即便它来自
/// 可信配置也要兜一道底 —— 拼接 SQL 的地方不该有「这个值肯定没问题」的假设。
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
            "环境变量 {name} 未设置。生成一把新密钥：cargo run -- keygen"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// schema 名会被拼进 `SET search_path`（那条语句不接受参数绑定），
    /// 所以校验是真正的注入防线，不是形式主义。
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
            assert!(!is_valid_ident(bad), "{bad:?} 应被拒绝");
        }
    }

    #[test]
    fn accepts_normal_schema_names() {
        for ok in ["ignition", "growing_tales", "_tmp", "s1"] {
            assert!(is_valid_ident(ok), "{ok:?} 应被接受");
        }
    }

    #[test]
    fn rejects_over_length_identifiers() {
        // Postgres 的标识符上限是 63 字节，超了会被静默截断 ——
        // 截断后可能撞上另一个已存在的 schema。
        assert!(is_valid_ident(&"a".repeat(63)));
        assert!(!is_valid_ident(&"a".repeat(64)));
    }
}
