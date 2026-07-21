.PHONY: help up down reset migrate migrate-remote seed secrets keygen build run test lint fmt psql \
        job-clear job-audit job-settle tma-install tma-dev tma-build tma-typecheck test-all

help:
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN{FS=":.*?## "}{printf "\033[36m%-14s\033[0m %s\n",$$1,$$2}'

# Local Postgres/Redis live under docker/; keep COMPOSE_FILE exported so plain
# `docker compose …` from make targets resolves the same file.
COMPOSE_FILE ?= docker/compose.yml
export COMPOSE_FILE
COMPOSE   = docker compose
PSQL      = $(COMPOSE) exec -T postgres psql -v ON_ERROR_STOP=1 -U postgres -d ignition
CFG       = configs/config.yaml
DB_SCHEMA ?= ignition

# Language workspaces live under apps/ — not the repository root.
# cargo requires --manifest-path after the subcommand.
APPS      = apps
MANIFEST  = --manifest-path $(APPS)/Cargo.toml
PKG       = -p ignition
PNPM      = pnpm --dir $(APPS)

up: ## Start postgres + redis
	$(COMPOSE) up -d --wait

down: ## Stop and remove data volumes
	$(COMPOSE) down -v

reset: down up migrate seed secrets ## Rebuild a clean database ready for end-to-end local runs

migrate: ## Run migrations (as admin)
	$(PSQL) < db/migrations/0001_init.sql
	$(PSQL) < db/migrations/0002_auth_game_postback.sql
	@# Migration creates ignition_app as NOLOGIN (runs on internet-reachable hosted DBs;
	@# cannot seed default passwords). Local docker is not exposed; enable login here for dev.
	@# In production, run this separately with a random password.
	$(PSQL) -c "ALTER ROLE ignition_app LOGIN PASSWORD 'ignition_app';"

migrate-remote: ## Run migrations on remote DB (reads IGNITION_PG_DSN; does not touch role passwords)
	@test -n "$$IGNITION_PG_DSN" || (echo "IGNITION_PG_DSN is required" && exit 1)
	psql "$$IGNITION_PG_DSN" -v ON_ERROR_STOP=1 -v schema=$(DB_SCHEMA) < db/migrations/0001_init.sql
	psql "$$IGNITION_PG_DSN" -v ON_ERROR_STOP=1 -v schema=$(DB_SCHEMA) < db/migrations/0002_auth_game_postback.sql

seed: ## Load demo data (tenant / KOL / campaign / prize pool / link)
	$(PSQL) < db/seed.sql

keygen: ## Generate a new master key
	@cargo run $(MANIFEST) $(PKG) -q -- keygen

# Bot token and API key are ciphertext — they must not live in seed.sql in the repo or
# encryption is theatre. `ignition seal` encrypts here; ciphertext exists only in local DB.
secrets: ## Write demo bot token and API key (requires IGNITION_MASTER_KEY)
	@BOT=$$(cargo run $(MANIFEST) $(PKG) -q -- $(CFG) seal '123456:AA-demo-bot-token'); \
	 KEY=$$(cargo run $(MANIFEST) $(PKG) -q -- $(CFG) seal 'demo-api-secret'); \
	 $(PSQL) -c "INSERT INTO bot (id, tenant_id, username, token_enc) \
	             VALUES (1, 1, 'demo_bot', '$$BOT') \
	             ON CONFLICT (id) DO UPDATE SET token_enc = EXCLUDED.token_enc; \
	             INSERT INTO api_key (id, tenant_id, key_id, secret_enc, label, scopes) \
	             VALUES (1, 1, 'ik_demo', '$$KEY', 'Demo main app', '{redeem,postback}') \
	             ON CONFLICT (id) DO UPDATE SET secret_enc = EXCLUDED.secret_enc; \
	             SELECT setval('bot_id_seq', 1), setval('api_key_id_seq', 1);"

build: ## Build API
	cargo build $(MANIFEST) $(PKG)

run: ## Run API locally
	cargo run $(MANIFEST) $(PKG) -- $(CFG)

job-clear:  ## Release events whose hold period has ended
	cargo run $(MANIFEST) $(PKG) -q -- $(CFG) job clear-holds
job-audit:  ## Ledger invariant audit
	cargo run $(MANIFEST) $(PKG) -q -- $(CFG) job ledger-audit
job-settle: ## End-of-month settlement
	cargo run $(MANIFEST) $(PKG) -q -- $(CFG) job settle

test: ## API unit tests (no database)
	cargo test $(MANIFEST) $(PKG)

lint: ## API clippy + fmt check
	cargo clippy $(MANIFEST) $(PKG) --all-targets -- -D warnings
	cargo fmt $(MANIFEST) --all -- --check

fmt: ## Format API
	cargo fmt $(MANIFEST) --all

psql: ## Open database shell
	$(COMPOSE) exec postgres psql -U postgres -d ignition

tma-install: ## Install frontend dependencies
	$(PNPM) install

tma-dev: ## Start TMA dev server
	$(PNPM) --filter @ignition/tma dev

tma-build: ## Build TMA
	$(PNPM) --filter @ignition/tma build

tma-typecheck: ## TMA typecheck
	$(PNPM) --filter @ignition/tma typecheck

test-all: test tma-typecheck ## API unit tests + TMA typecheck
