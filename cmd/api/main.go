// Command api 是 LinkSprout 的 HTTP 服务入口。
package main

import (
	"context"
	"flag"
	"log/slog"
	"os"
	"os/signal"
	"syscall"

	"github.com/shaoxiaoxu/linksprout/internal/conf"
	"github.com/shaoxiaoxu/linksprout/internal/dao"
	"github.com/shaoxiaoxu/linksprout/internal/server"
	"github.com/shaoxiaoxu/linksprout/internal/service/attribution"
)

func main() {
	configPath := flag.String("config", "configs/config.yaml", "配置文件路径")
	flag.Parse()

	log := slog.New(slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{Level: slog.LevelInfo}))

	cfg, err := conf.Load(*configPath)
	if err != nil {
		log.Error("加载配置失败", "err", err)
		os.Exit(1)
	}

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	db, err := dao.Open(ctx, cfg.Postgres.DSN)
	if err != nil {
		log.Error("连接数据库失败", "err", err)
		os.Exit(1)
	}
	defer db.Close()

	policy, err := attribution.PolicyByVersion(cfg.Attribution.PolicyVersion)
	if err != nil {
		log.Error("归因策略版本未知", "err", err)
		os.Exit(1)
	}

	srv := server.New(cfg, db, attribution.NewService(db, policy), log)
	if err := srv.Run(ctx); err != nil {
		log.Error("服务退出", "err", err)
		os.Exit(1)
	}
}
