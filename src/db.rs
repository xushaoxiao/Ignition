//! PostgreSQL 连接与租户事务。

use sqlx::postgres::{PgPoolOptions, Postgres};
use sqlx::{PgPool, Transaction};

/// 建立连接池。
pub async fn connect(dsn: &str, max_connections: u32) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(dsn)
        .await
}

/// 开启事务并设置 RLS 所需的租户上下文。
///
/// `set_config` 的第三个参数为 `true` 等价于 `SET LOCAL`，作用域是事务，
/// 事务结束后自动失效，因此不会泄漏到连接池里被下一个租户的请求复用 ——
/// **这一点至关重要**，用 `SET`（而非 `SET LOCAL`）会造成跨租户数据泄漏。
///
/// 所有租户数据的读写都必须走这里。迁移里的 RLS 策略用
/// `current_setting('app.tenant_id', true)`，未设置时返回 NULL，比较结果为
/// false，因此漏设租户的查询会读到空集而不是全表 —— fail-closed。
pub async fn begin_tenant_tx(
    pool: &PgPool,
    tenant_id: i64,
) -> Result<Transaction<'static, Postgres>, sqlx::Error> {
    debug_assert!(tenant_id > 0, "tenant_id 必须为正");

    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant_id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}
