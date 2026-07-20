//! Ignition —— 私域游戏化增长与全链路归因平台。
//!
//! 这个系统的本质不是「游戏化增长工具」，而是一台能出具可信账单的归因机器。
//! 转盘是获客话术，归因链路和账本才是产品。
//!
//! ```text
//! ignition <config>                启动 HTTP 服务
//! ignition <config> job <name>     跑一次定时任务
//! ignition <config> seal <明文>     加密一段密钥，输出可直接入库的 bytea 字面量
//! ignition keygen                  生成一把新的主密钥
//! ```

mod attribution;
mod auth;
mod billing;
mod config;
mod db;
mod entitlement;
mod game;
mod hmacsig;
mod jobs;
mod ledger;
mod models;
mod risk;
mod secrets;
mod server;
mod telegram;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use chrono::Utc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // keygen 在加载配置之前处理，且不挑位置：一个全新的部署还没有密钥，
    // 正是要靠它生成。要求它先有密钥才能跑，是个死循环。
    if args.iter().any(|a| a == "keygen") {
        println!("{}", secrets::generate_master_key_b64());
        return Ok(());
    }

    init_tracing();

    let config_path = args
        .first()
        .cloned()
        .unwrap_or_else(|| "configs/config.yaml".to_string());
    let cfg = config::Config::load(&config_path)?;
    let policy = attribution::by_version(&cfg.attribution.policy_version)?;
    let cipher = secrets::Cipher::from_master_key_b64(&cfg.secrets.master_key_b64)
        .context("IGNITION_MASTER_KEY 非法")?;

    match args.get(1).map(String::as_str) {
        Some("seal") => {
            let plaintext = args.get(2).context("用法: ignition <config> seal <明文>")?;
            // 输出成 Postgres 的 bytea 字面量，可以直接贴进 SQL。
            println!("\\x{}", hex::encode(cipher.seal(plaintext.as_bytes())));
            Ok(())
        }
        Some("job") => {
            let name = args.get(2).context("用法: ignition <config> job <name>")?;
            let pool = connect_db(&cfg).await?;
            run_job(name, &pool).await
        }
        _ => serve(cfg, policy, cipher).await,
    }
}

/// 建连接池，并在启动时确认 RLS 真的会生效。
///
/// 这个检查不能省。租户隔离失效是**没有症状**的：接口照常返回、测试照常通过，
/// 只有当某个客户看到别人的数据时才会暴露。宁可每次启动多一次往返。
async fn connect_db(cfg: &config::Config) -> anyhow::Result<sqlx::PgPool> {
    let pool = db::connect(
        &cfg.postgres.dsn,
        cfg.postgres.max_connections,
        &cfg.postgres.schema,
    )
    .await
    .context("连接数据库失败")?;

    // 把目标库打进启动日志。「连上了」和「连对了」是两回事：漏 source 一个
    // env 文件就会静默连到本地空库，然后花半小时排查「活动不存在」。
    let target = db::describe(&cfg.postgres.dsn);

    if db::bypasses_rls(&pool).await? {
        tracing::error!(
            target_db = %target,
            schema = %cfg.postgres.schema,
            "当前数据库角色带 BYPASSRLS/SUPERUSER，租户隔离（约束 C5）已失效；\
             应改用专用的非特权角色连接，见 README「共享 Supabase」"
        );
    } else {
        tracing::info!(target_db = %target, schema = %cfg.postgres.schema, "已连接数据库，RLS 生效");
    }
    Ok(pool)
}

async fn run_job(name: &str, pool: &sqlx::PgPool) -> anyhow::Result<()> {
    let now = Utc::now();
    match name {
        "clear-holds" => {
            let r = jobs::clear::run(pool, now).await?;
            tracing::info!(scanned = r.scanned, cleared = r.cleared, "冷静期放行完成");
        }
        "ledger-audit" => {
            let violations = jobs::audit::run(pool).await?;
            if !violations.is_empty() {
                // 非零退出码：让调度器把它当作失败，从而触发告警。账本对不上
                // 意味着已开出的账单可能是错的，这不是一条能埋在日志里的信息。
                anyhow::bail!("账本不变量被破坏，共 {} 条", violations.len());
            }
            tracing::info!("账本校验通过");
        }
        "settle" => {
            let period = jobs::settle::Period::previous_month(now);
            jobs::settle::run(pool, period, now).await?;
        }
        other => anyhow::bail!("未知任务 {other:?}，可选：clear-holds / ledger-audit / settle"),
    }
    Ok(())
}

async fn serve(
    cfg: config::Config,
    policy: attribution::policy::Policy,
    cipher: secrets::Cipher,
) -> anyhow::Result<()> {
    let pool = connect_db(&cfg).await?;

    let state = Arc::new(server::AppState {
        attribution: attribution::Service::new(pool.clone(), policy),
        issue: attribution::issue::Service::new(pool.clone(), policy),
        postback: attribution::postback::Service::new(pool.clone(), policy),
        game: game::play::Service::new(pool.clone()),
        issuer: auth::jwt::Issuer::new(cfg.secrets.jwt_key.as_bytes()),
        cipher,
        pool,
    });

    let addr: SocketAddr = cfg.http.addr.parse().context("http.addr 格式非法")?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, policy = policy.version, "服务启动");

    axum::serve(
        listener,
        server::router(state, &cfg.http.cors_origins)
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("收到中断信号，开始优雅退出");
}
