// Package dao 封装 PostgreSQL 访问。
package dao

import (
	"context"
	"fmt"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgxpool"
)

// DB 是数据库句柄。
type DB struct{ Pool *pgxpool.Pool }

// Open 建立连接池。
func Open(ctx context.Context, dsn string) (*DB, error) {
	cfg, err := pgxpool.ParseConfig(dsn)
	if err != nil {
		return nil, fmt.Errorf("dao: 解析 DSN 失败: %w", err)
	}
	pool, err := pgxpool.NewWithConfig(ctx, cfg)
	if err != nil {
		return nil, fmt.Errorf("dao: 建立连接池失败: %w", err)
	}
	if err := pool.Ping(ctx); err != nil {
		return nil, fmt.Errorf("dao: ping 失败: %w", err)
	}
	return &DB{Pool: pool}, nil
}

func (d *DB) Close() { d.Pool.Close() }

// InTenantTx 在一个事务内执行 fn，并为该事务设置 RLS 所需的租户上下文。
//
// SET LOCAL 的作用域是事务，事务结束后自动失效，因此不会泄漏到连接池里被
// 下一个租户的请求复用 —— 这一点至关重要，用 SET（而非 SET LOCAL）会造成
// 跨租户数据泄漏。
//
// 所有租户数据的读写都必须走这里。迁移里的 RLS 策略用
// current_setting('app.tenant_id', true)，未设置时返回 NULL，比较结果为
// false，因此漏设 tenant 的查询会读到空集而不是全表 —— fail-closed。
func (d *DB) InTenantTx(ctx context.Context, tenantID int64, fn func(pgx.Tx) error) error {
	if tenantID <= 0 {
		return fmt.Errorf("dao: 非法 tenantID %d", tenantID)
	}
	tx, err := d.Pool.Begin(ctx)
	if err != nil {
		return fmt.Errorf("dao: 开启事务失败: %w", err)
	}
	defer func() { _ = tx.Rollback(ctx) }()

	// set_config 的第三个参数 true 等价于 SET LOCAL。
	if _, err := tx.Exec(ctx, "SELECT set_config('app.tenant_id', $1, true)",
		fmt.Sprint(tenantID)); err != nil {
		return fmt.Errorf("dao: 设置租户上下文失败: %w", err)
	}
	if err := fn(tx); err != nil {
		return err
	}
	return tx.Commit(ctx)
}
