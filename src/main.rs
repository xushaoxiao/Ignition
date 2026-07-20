//! Growth —— 私域游戏化增长与全链路归因平台。
//!
//! 这个系统的本质不是「游戏化增长工具」，而是一台能出具可信账单的归因机器。
//! 转盘是获客话术，归因链路和账本才是产品。

// 账本、封顶、回传签名已实现并有测试覆盖，但尚未接入在线链路 —— 它们要等
// 月末结算任务和 /v1/postback/purchase 落地（见 README「未实现」清单）。
// 这个 allow 是有期限的：那两块接完之后应当移除，让 dead_code 重新变成信号。
#![allow(dead_code)]

mod attribution;
mod billing;
mod config;
mod db;
mod hmacsig;
mod ledger;
mod models;
mod risk;
mod server;
mod telegram;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "configs/config.yaml".to_string());

    let cfg = config::Config::load(&config_path)?;
    let policy = attribution::by_version(&cfg.attribution.policy_version)?;

    let pool = db::connect(&cfg.postgres.dsn, cfg.postgres.max_connections)
        .await
        .context("连接数据库失败")?;

    let state = Arc::new(server::AppState {
        attribution: attribution::Service::new(pool.clone(), policy),
        pool,
    });

    let addr: SocketAddr = cfg.http.addr.parse().context("http.addr 格式非法")?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, policy = policy.version, "服务启动");

    axum::serve(
        listener,
        server::router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("收到中断信号，开始优雅退出");
}
