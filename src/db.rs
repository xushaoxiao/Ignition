//! PostgreSQL 连接与租户事务。
//!
//! 所有对象都落在一个独立 schema（默认 `ignition`），通过每条连接的
//! `search_path` 隔离 —— 与 growing-tales 共用一个 Supabase 实例时，
//! 这是两个项目互不干扰的唯一保证。

use sqlx::postgres::{PgConnectOptions, PgPoolOptions, Postgres};
use sqlx::{Executor, PgPool, Transaction};

/// 建立连接池，并把目标 schema 放到每条连接的 `search_path` 首位。
///
/// 用 `after_connect` 而不是在 DSN 里写 `options=-csearch_path=...`：
/// 连接池会不断重建连接，钩子对每条新连接都生效；而 DSN 里的 options 参数
/// 在 Supabase 的 Supavisor 这类连接池后面并不可靠。
///
/// `search_path` 保留 `public` 兜底 —— `gen_random_uuid` 这类扩展函数装在
/// public / extensions 里，去掉会等到运行时才炸。
pub async fn connect(dsn: &str, max_connections: u32, schema: &str) -> Result<PgPool, sqlx::Error> {
    // schema 已在 config 层校验过是合法标识符，这里可以安全拼接。
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

/// 开启事务并设置 RLS 所需的租户上下文。
///
/// `set_config` 的第三个参数为 `true` 等价于 `SET LOCAL`，作用域是事务，
/// 事务结束后自动失效，因此不会泄漏到连接池里被下一个租户的请求复用 ——
/// **这一点至关重要**，用 `SET`（而非 `SET LOCAL`）会造成跨租户数据泄漏。
///
/// 所有租户数据的读写都必须走这里。迁移里的 RLS 策略用
/// `current_setting('app.tenant_id', true)`，未设置时返回 NULL，比较结果为
/// false，因此漏设租户的查询会读到空集而不是全表 —— fail-closed。
///
/// **前提是连接角色不带 `BYPASSRLS`。** 用 Supabase 的 `postgres` 角色直连时
/// 这个前提不成立，RLS 会被整体绕过 —— 见 README 的「共享 Supabase」一节。
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

/// 从 DSN 里取出 `host:port/dbname`，用于启动日志。
///
/// **绝不返回用户名和口令。** 这个字符串会进日志，而日志会被复制到工单、
/// 聊天窗口和监控系统里。
pub fn describe(dsn: &str) -> String {
    match dsn.parse::<PgConnectOptions>() {
        Ok(o) => format!(
            "{}:{}/{}",
            o.get_host(),
            o.get_port(),
            o.get_database().unwrap_or("?")
        ),
        Err(_) => "<无法解析的 DSN>".into(),
    }
}

/// 连接角色是否会绕过 RLS。
///
/// 启动时查一次并告警。这个检查存在的理由很具体：把服务指向一个带
/// `BYPASSRLS` 的角色（Supabase 的 `postgres` 正是）之后，功能一切正常、
/// 测试全过 —— 唯独租户隔离静默失效。**没有任何症状的安全退化最危险**，
/// 所以必须在启动日志里喊出来，而不是等某天客户看到别人的数据。
pub async fn bypasses_rls(pool: &PgPool) -> Result<bool, sqlx::Error> {
    let row: (bool,) = sqlx::query_as(
        "SELECT COALESCE(bool_or(rolbypassrls OR rolsuper), false)
           FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}
