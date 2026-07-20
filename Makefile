.PHONY: help up down migrate seed build run test test-integration lint tidy psql

PG_ADMIN_DSN ?= postgres://postgres:postgres@localhost:55432/linksprout?sslmode=disable

help:
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*?## "}{printf "\033[36m%-18s\033[0m %s\n",$$1,$$2}'

up: ## 启动 postgres + redis
	docker compose up -d --wait

down: ## 停止并清理
	docker compose down -v

migrate: ## 执行迁移（以 admin 身份）
	docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U postgres -d linksprout < migration/0001_init.sql

seed: ## 灌入演示数据（一个租户 + KOL + 活动 + 投放位）
	docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U postgres -d linksprout < migration/seed.sql

build: ## 编译
	go build -o bin/api ./cmd/api

run: ## 本地运行
	go run ./cmd/api -config configs/config.yaml

test: ## 单元测试（不需要 DB）
	go test ./... -count=1

test-integration: ## 集成测试（需先 make up migrate）
	LINKSPROUT_TEST_DSN="$(PG_ADMIN_DSN)" go test ./... -count=1 -tags=integration

lint:
	go vet ./...

tidy:
	go mod tidy

psql: ## 进入数据库
	docker compose exec postgres psql -U postgres -d linksprout
