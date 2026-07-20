//! 运行配置。

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub http: Http,
    pub postgres: Postgres,
    #[serde(default)]
    pub attribution: Attribution,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Http {
    #[serde(default = "default_addr")]
    pub addr: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Postgres {
    pub dsn: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
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

        if let Ok(dsn) = std::env::var("LINKSPROUT_PG_DSN") {
            cfg.postgres.dsn = dsn;
        }
        Ok(cfg)
    }
}
