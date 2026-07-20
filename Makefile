.PHONY: help up down reset migrate seed build run test lint fmt psql

help:
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*?## "}{printf "\033[36m%-12s\033[0m %s\n",$$1,$$2}'

up: ## 启动 postgres + redis
	docker compose up -d --wait

down: ## 停止并清理数据卷
	docker compose down -v

reset: down up migrate seed ## 重建一个干净的数据库

migrate: ## 执行迁移（以 admin 身份）
	docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U postgres -d ignition < migration/0001_init.sql

seed: ## 灌入演示数据（一个租户 + KOL + 活动 + 领奖码）
	docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U postgres -d ignition < migration/seed.sql

build:
	cargo build

run: ## 本地运行
	cargo run -- configs/config.yaml

test: ## 单元测试，不需要数据库
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings
	cargo fmt --check

fmt:
	cargo fmt

psql: ## 进入数据库
	docker compose exec postgres psql -U postgres -d ignition
