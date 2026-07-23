//! Ignition — private-domain gamified growth and full-funnel attribution platform.
//!
//! The system's essence is not a "gamified growth tool" but a machine that produces trustworthy
//! invoices. The wheel is the acquisition pitch; attribution chain and ledger are the product.
//!
//! ```text
//! ignition <config>                Start HTTP service
//! ignition <config> job <name>     Run a scheduled job once
//! ignition <config> seal <plaintext>  Encrypt secret material; output Postgres bytea literal
//! ignition keygen                  Generate a new master key
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

    // Handle keygen before loading config, from any cwd: a fresh deploy has no keys yet and
    // requiring keys to run keygen is circular.
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
        .context("IGNITION_MASTER_KEY invalid")?;

    match args.get(1).map(String::as_str) {
        Some("seal") => {
            let plaintext = args
                .get(2)
                .context("usage: ignition <config> seal <plaintext>")?;
            // Emit a Postgres bytea literal suitable for pasting into SQL.
            println!("\\x{}", hex::encode(cipher.seal(plaintext.as_bytes())));
            Ok(())
        }
        Some("job") => {
            let name = args.get(2).context("usage: ignition <config> job <name>")?;
            let pool = connect_db(&cfg).await?;
            run_job(name, &pool).await
        }
        _ => serve(cfg, policy, cipher).await,
    }
}

/// Create the pool and confirm RLS will actually apply at startup.
///
/// Do not skip this check. Tenant isolation failure is **symptomless**: APIs respond, tests pass,
/// until a customer sees someone else's data. One extra round-trip at startup is cheap.
async fn connect_db(cfg: &config::Config) -> anyhow::Result<sqlx::PgPool> {
    let pool = db::connect(
        &cfg.postgres.dsn,
        cfg.postgres.max_connections,
        &cfg.postgres.schema,
    )
    .await
    .context("failed to connect to database")?;

    // Log target database — "connected" and "connected to the right place" differ; a missing
    // env file silently points at an empty local DB and you spend half an hour on "campaign not found".
    let target = db::describe(&cfg.postgres.dsn);

    if db::bypasses_rls(&pool).await? {
        tracing::error!(
            target_db = %target,
            schema = %cfg.postgres.schema,
            "database role has BYPASSRLS/SUPERUSER; tenant isolation (constraint C5) is disabled. \
             Connect with a dedicated non-privileged role instead — see README \"Shared Supabase\""
        );
    } else {
        tracing::info!(target_db = %target, schema = %cfg.postgres.schema, "connected to database, RLS active");
    }
    Ok(pool)
}

async fn run_job(name: &str, pool: &sqlx::PgPool) -> anyhow::Result<()> {
    let now = Utc::now();
    match name {
        "clear-holds" => {
            let r = jobs::clear::run(pool, now).await?;
            tracing::info!(
                scanned = r.scanned,
                cleared = r.cleared,
                "hold clearance complete"
            );
        }
        "ledger-audit" => {
            let violations = jobs::audit::run(pool).await?;
            if !violations.is_empty() {
                // Non-zero exit so schedulers alert — ledger mismatch means issued invoices may be wrong;
                // not something to bury in logs.
                anyhow::bail!(
                    "ledger invariant violated, {} violation(s)",
                    violations.len()
                );
            }
            tracing::info!("ledger audit passed");
        }
        "settle" => {
            let period = jobs::settle::Period::previous_month(now);
            jobs::settle::run(pool, period, now).await?;
        }
        other => {
            anyhow::bail!("unknown job {other:?}; options: clear-holds / ledger-audit / settle")
        }
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

    let addr: SocketAddr = cfg.http.addr.parse().context("invalid http.addr format")?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, policy = policy.version, "server starting");

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
    tracing::info!("interrupt received, shutting down gracefully");
}
