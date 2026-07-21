//! PostgreSQL connections and tenant transactions.
//!
//! All objects live in a dedicated schema (default `ignition`), isolated via each
//! connection's `search_path` — when sharing a Supabase instance with growing-tales,
//! this is the only guarantee the two projects do not interfere.

use sqlx::postgres::{PgConnectOptions, PgPoolOptions, Postgres};
use sqlx::{Executor, PgPool, Transaction};

/// Create a pool and put the target schema first on every connection's `search_path`.
///
/// Uses `after_connect` rather than `options=-csearch_path=...` in the DSN: the pool recreates
/// connections over time and the hook runs on each new one; DSN options are unreliable behind
/// Supabase Supavisor-style poolers.
///
/// Keeps `public` on `search_path` as a fallback — extension helpers like `gen_random_uuid`
/// live in public/extensions; dropping it fails at runtime.
pub async fn connect(dsn: &str, max_connections: u32, schema: &str) -> Result<PgPool, sqlx::Error> {
    // Schema was validated as a legal identifier in config; safe to splice here.
    let set_path = format!("SET search_path TO {schema}, public");
    let opts: PgConnectOptions = dsn.parse()?;

    PgPoolOptions::new()
        .max_connections(max_connections)
        .after_connect(move |conn, _meta| {
            let set_path = set_path.clone();
            Box::pin(async move {
                conn.execute(set_path.as_str()).await?;
                Ok(())
            })
        })
        .connect_with(opts)
        .await
}

/// Begin a transaction and set tenant context required by RLS.
///
/// The third argument `true` to `set_config` is equivalent to `SET LOCAL`: scoped to the
/// transaction and automatically cleared on commit/rollback, so it cannot leak into the
/// pool for the next tenant — **this is critical**; plain `SET` (not `SET LOCAL`) causes
/// cross-tenant data leaks.
///
/// All tenant data reads and writes must go through here. RLS policies in migrations use
/// `current_setting('app.tenant_id', true)`; when unset it returns NULL, comparisons fail,
/// and queries return empty sets rather than the whole table — fail-closed.
///
/// **Requires a connection role without `BYPASSRLS`.** Connecting as Supabase's `postgres`
/// role violates that premise and RLS is bypassed entirely — see README "Shared Supabase".
pub async fn begin_tenant_tx(
    pool: &PgPool,
    tenant_id: i64,
) -> Result<Transaction<'static, Postgres>, sqlx::Error> {
    debug_assert!(tenant_id > 0, "tenant_id must be positive");

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// Extract `host:port/dbname` from the DSN for startup logs.
///
/// **Never returns username or password.** This string goes into logs copied to tickets,
/// chat, and monitoring systems.
pub fn describe(dsn: &str) -> String {
    match dsn.parse::<PgConnectOptions>() {
        Ok(o) => format!(
            "{}:{}/{}",
            o.get_host(),
            o.get_port(),
            o.get_database().unwrap_or("?")
        ),
        Err(_) => "<unparseable DSN>".into(),
    }
}

/// Whether the connection role bypasses RLS.
///
/// Checked once at startup with an alert. The concrete reason: pointing the service at a
/// `BYPASSRLS` role (Supabase `postgres` does) looks fine — features work, tests pass —
/// except tenant isolation silently fails. **Security regressions with no symptoms are the
/// most dangerous**, so shout in startup logs rather than waiting until a customer sees
/// someone else's data.
pub async fn bypasses_rls(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let row: (bool,) = sqlx::query_as(
        "SELECT COALESCE(bool_or(rolbypassrls OR rolsuper), false)
           FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}
