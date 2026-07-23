# Ignition API

Rust HTTP service, background jobs, and CLI tools (`keygen`, `seal`, …).

Package name: `ignition`. Cargo workspace root is `apps/` (sibling of this crate). Prefer **repository-root** `make` targets so config paths stay correct.

## Local run

```bash
# from repo root
cp configs/config.example.yaml configs/config.yaml
export IGNITION_MASTER_KEY=$(cargo run --manifest-path apps/Cargo.toml -p ignition -q -- keygen)
export IGNITION_JWT_KEY=$(openssl rand -base64 32)

make reset
make run
```

See the root [README.md](../../README.md) for Supabase notes, constraints, and product context.
Hard rules for changes: [CLAUDE.md](../../CLAUDE.md).

## Layout

```
src/
  main.rs             entry: serve / jobs / key tools
  config.rs           config load (secrets from env only)
  models.rs           domain types, attribution billability, state machine, Cents
  db.rs               pool, tenant tx (RLS)
  secrets.rs          _enc encrypt/decrypt + Secret wrapper
  ledger.rs           double-entry ledger
  billing.rs          transitions, monthly cap
  entitlement.rs      capability gating
  risk.rs             L1 hard checks
  telegram.rs         initData verify
  hmacsig.rs          HMAC + time window
  auth/               API Key (S2S), JWT (TMA)
  attribution/        policy, claim codes, redeem, postback, query
  game/               authoritative play + inventory
  payments.rs         payment gateway seam (invoice push); log gateway + Stripe drop-in
  jobs/               clear-holds, ledger-audit, settle, push-invoices
  server/             HTTP handlers
```

Migrations live in [`db/migrations/`](../../db/migrations/), not in this package.
