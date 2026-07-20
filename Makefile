.PHONY: help up down reset migrate seed secrets keygen build run test lint fmt psql \
        job-clear job-audit job-settle

help:
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*?## "}{printf "\033[36m%-12s\033[0m %s\n",$$1,$$2}'

PSQL      = docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U postgres -d ignition
CFG       = configs/config.yaml
DB_SCHEMA ?= ignition

up: ## 启动 postgres + redis
	docker compose up -d --wait

down: ## 停止并清理数据卷
	docker compose down -v

reset: down up migrate seed secrets ## 重建一个干净的、可直接跑通全链路的数据库

migrate: ## 执行迁移（以 admin 身份）
	$(PSQL) < migration/0001_init.sql
	$(PSQL) < migration/0002_auth_game_postback.sql
	@# 迁移把 ignition_app 建成 NOLOGIN 的权限容器（它会在公网可达的托管库上
	@# 执行，不能种默认口令）。本地 docker 不对外，这里顺手开登录方便开发。
	@# 生产环境请用随机口令单独执行这一句。
	$(PSQL) -c "ALTER ROLE ignition_app LOGIN PASSWORD 'ignition_app';"

migrate-remote: ## 对远端库执行迁移（读 IGNITION_PG_DSN，不碰角色口令）
	@test -n "$$IGNITION_PG_DSN" || (echo "需要 IGNITION_PG_DSN" && exit 1)
	psql "$$IGNITION_PG_DSN" -v ON_ERROR_STOP=1 -v schema=$(DB_SCHEMA) < migration/0001_init.sql
	psql "$$IGNITION_PG_DSN" -v ON_ERROR_STOP=1 -v schema=$(DB_SCHEMA) < migration/0002_auth_game_postback.sql

seed: ## 灌入演示数据（租户 / KOL / 活动 / 奖池 / 投放位）
	$(PSQL) < migration/seed.sql

keygen: ## 生成一把新的主密钥
	@cargo run -q -- keygen

# Bot token 与 API Key 是密文，不能写进版本库里的 seed.sql —— 那样加密存储就
# 只是个摆设。这里用 `ignition seal` 现场加密，密文只存在于本地数据库。
secrets: ## 写入演示 Bot token 与 API Key（需要 IGNITION_MASTER_KEY）
	@BOT=$$(cargo run -q -- $(CFG) seal '123456:AA-demo-bot-token'); \
	 KEY=$$(cargo run -q -- $(CFG) seal 'demo-api-secret'); \
	 $(PSQL) -c "INSERT INTO bot (id, tenant_id, username, token_enc) \
	             VALUES (1, 1, 'demo_bot', '$$BOT') \
	             ON CONFLICT (id) DO UPDATE SET token_enc = EXCLUDED.token_enc; \
	             INSERT INTO api_key (id, tenant_id, key_id, secret_enc, label, scopes) \
	             VALUES (1, 1, 'ik_demo', '$$KEY', 'Demo main app', '{redeem,postback}') \
	             ON CONFLICT (id) DO UPDATE SET secret_enc = EXCLUDED.secret_enc; \
	             SELECT setval('bot_id_seq', 1), setval('api_key_id_seq', 1);"

build:
	cargo build

run: ## 本地运行
	cargo run -- $(CFG)

job-clear:  ## 冷静期到期放行
	cargo run -q -- $(CFG) job clear-holds
job-audit:  ## 账本不变量校验
	cargo run -q -- $(CFG) job ledger-audit
job-settle: ## 月末结算
	cargo run -q -- $(CFG) job settle

test: ## 单元测试，不需要数据库
	cargo test

lint:
	cargo clippy --all-targets -- -D warnings
	cargo fmt --check

fmt:
	cargo fmt

psql: ## 进入数据库
	docker compose exec postgres psql -U postgres -d ignition
