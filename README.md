# Ignition

Private-domain gamified growth with end-to-end attribution.

**This product is not a “gamification growth toolkit”. It is an attribution machine that can issue trustworthy invoices.**
The wheel is acquisition theatre; the attribution path and the ledger are the product. Engineering priorities follow that judgement.

## Docs

| Doc | Contents | Audience |
|---|---|---|
| [docs/design/system-design.md](docs/design/system-design.md) | Full system design: business model → capabilities, domain model, attribution, billing & ledger, risk, milestones | Internal |
| [docs/product/attribution-policy-v1.md](docs/product/attribution-policy-v1.md) | Attribution policy v1 | **Customers & KOLs** |
| [docs/engineering/monorepo.md](docs/engineering/monorepo.md) | Repo layout and how to add an app | Internal |
| [CLAUDE.md](CLAUDE.md) | Hard rules before changing code | Internal |

Doc index: [docs/README.md](docs/README.md).

The system design describes the **target** state. The “Current status” section below is what is actually shipping. When they conflict, this file wins — then go fix the design doc.

---

## Current status

The revenue path runs end to end: **TMA open → play → claim code → redeem / attribute → billable event → month-end invoice → double-entry ledger → ledger audit**. Several pre-launch items remain; see “Not yet shipped”.

### Shipped

| Module | Location | Notes |
|---|---|---|
| Domain model & migrations | `db/migrations/0001_init.sql` | RLS tenant isolation; ledger UPDATE/DELETE revoked |
| Attribution methods & billability | `apps/api/src/models.rs` | Only deterministic methods are billable; exhaustive `match` forces a stance |
| Billable-event state machine | `apps/api/src/models.rs` + `billing.rs` | pending → cleared → billed → reversed |
| Double-entry ledger | `apps/api/src/ledger.rs` | Unbalanced `Txn` is unrepresentable at the type level |
| Monthly cap | `apps/api/src/billing.rs` | Over-cap is free, not rejected |
| Claim codes | `apps/api/src/attribution/claim_code.rs` | Excludes confusable characters; normalise + format checks |
| Redeem transaction | `apps/api/src/attribution/redeem.rs` | Sole stitch point of the full path; single transaction |
| Versioned attribution policy | `apps/api/src/attribution/policy.rs` + `docs/product/attribution-policy-v1.md` | Recomputable; appeal-ready evidence model |
| TG initData verify | `apps/api/src/telegram.rs` | HMAC + freshness + per-tenant bot token |
| Postback signature verify | `apps/api/src/hmacsig.rs` | HMAC + timestamp window against replay |
| Risk L1 + L2 collection | `apps/api/src/risk.rs` | On redeem: prefer hold over deny |
| HTTP service | `apps/api/src/server/` | Two S2S endpoints + four TMA endpoints |
| **API Key + HMAC auth** | `apps/api/src/auth/apikey.rs` | Signs method + path + body; missing scope denies |
| **Encrypted secrets at rest** | `apps/api/src/secrets.rs` | AES-256-GCM; `Secret` Debug is always `<redacted>` |
| **TMA session** | `apps/api/src/auth/jwt.rs` | initData → access 15m + refresh 7d |
| **Server-authoritative play** | `apps/api/src/game/` | Weighted draw + atomic stock + idempotency |
| **Claim-code issue** | `apps/api/src/attribution/issue.rs` | One code per play; dual-platform landing guidance |
| **Monetisation postback** | `apps/api/src/attribution/postback.rs` | Analytics stream separated from billing stream |
| **Entitlement gating** | `apps/api/src/entitlement.rs` | Plan defaults + tenant override; default off |
| **Scheduled jobs** | `apps/api/src/jobs/` | Clear holds / ledger audit / month-end settle |
| **TMA frontend** | `apps/tma/` | React + Vite + Tailwind; wheel via CSS transform |

### Not yet shipped (priority order)

1. **Master key from env, not yet KMS** — ciphertext format in `apps/api/src/secrets.rs` already carries a version byte; KMS is a `V2` branch, old blobs stay readable, no downtime migration.
2. **Stripe** — month-end already builds `invoice` + `invoice_line` + ledger entries, but nothing is pushed to payments; `invoice.status` stays `draft`.
3. **Entitlement has few gate points** — capability set and subscription service levels exist and are tested; besides “stop issuing new sessions after past-due grace”, little is wired yet.
4. Detail export / diff view / appeal channel (design §5.4 — product features, not internal tools)
5. Reversal path: `ledger::Txn::reverse` exists and is tested; nothing triggers it yet
6. KOL console, three-metric dashboard, ClickHouse analytics stream
7. L3 channel-level risk scan
8. Attribution query API `GET /v1/attribution/:app_user_id`

---

## Quick start

### API

```bash
cp configs/config.example.yaml configs/config.yaml

# Both keys are env-only; missing keys fail startup — no defaults.
# A default signing key is a shared key across every deploy.
export IGNITION_MASTER_KEY=$(cargo run --manifest-path apps/Cargo.toml -p ignition -q -- keygen)
export IGNITION_JWT_KEY=$(openssl rand -base64 32)

make reset       # DB up + migrate + demo data + demo secrets
make run
```

The `secrets` step in `make reset` uses `ignition seal` to encrypt the demo Bot token and API Key at runtime — ciphertext must not live in the repo, or encrypted storage is theatre.

### Shared Supabase

When sharing a Supabase project with growing-tales, isolation is an **independent schema** (default `ignition`; `postgres.schema` / `IGNITION_PG_SCHEMA`). Each connection puts it first on `search_path` — same convention as growing-tales.

```bash
export IGNITION_PG_DSN='postgresql://...pooler.supabase.com:5432/postgres?sslmode=require'
make migrate-remote
```

Use the **Session Pooler (5432), not the Transaction Pooler (6543)**: the latter does not keep session state, so sqlx prepared statements and `search_path` break.

**The connection role must be `ignition_app`, never `postgres`.** Migrations create that role as a NOLOGIN privilege container; enable login separately at deploy time:

```sql
ALTER ROLE ignition_app LOGIN PASSWORD '<random>';
```

In the DSN, Supavisor usernames look like `ignition_app.<project-ref>` (verified). Prefer alphanumeric passwords — they go into the URL, and special characters need escaping.

After switching, startup logs `connected; RLS active`. An ERROR-level line means you are still on a privileged role.

> **Why `postgres` is forbidden: tenant isolation (C5) does not apply.**
>
> That role has `rolbypassrls`, so RLS policies are ignored — even `FORCE ROW LEVEL SECURITY` only constrains the table owner, not BYPASSRLS.
>
> There is **no symptom**: APIs succeed, tests pass; you only learn when a customer sees another tenant’s data. Startup checks `pg_roles` and logs ERROR, but that is a warning, not a fix.
>
> `ignition_app` is created NOLOGIN on purpose: migrations run against internet-reachable managed DBs; a login role with a default password is a shared door.

Append-only ledgers (C3) on managed DBs also use a `BEFORE UPDATE OR DELETE` trigger that applies to every role — including owner and BYPASSRLS — because `REVOKE` alone does not stop the table owner.

### Scheduled jobs

Jobs are pulled by an external scheduler on purpose: billing jobs must be re-runnable by hand with their own logs.

```bash
make job-clear     # hourly: move hold-expired events to cleared
make job-audit     # daily: ledger invariants; non-zero exit on failure (alert)
make job-settle    # monthly T+1: invoice the previous month
```

### TMA frontend

```bash
make tma-install   # pnpm --dir apps install
cp apps/tma/.env.example apps/tma/.env.local     # set VITE_API_BASE
make tma-dev
```

Telegram only loads HTTPS pages; local device debugging needs a tunnel (cloudflared / ngrok). Without a tunnel, the browser path uses a freshly signed initData from the Vite plugin — see [apps/tma/README.md](apps/tma/README.md).

### Tests

```bash
make test        # cargo test -p ignition via apps/Cargo.toml; no database required
make lint        # cargo clippy + fmt --check
make test-all    # API unit tests + TMA typecheck
```

---

## Four non-negotiable constraints

Many design choices only make sense against these. Read them before changing code.

### C1 Billing depends only on deterministic attribution

Probabilistic conversions may appear on dashboards; they must never appear on invoices. `AttributionMethod::is_billable()` is the sole enforcement point — an exhaustive `match`, not a lookup table. Adding a method forces an explicit stance at compile time; forgetting to register cannot silently fall through. `models.rs` tests guard this again.

Background: after iOS 17+, user-level deferred deep linking has no reliable implementation. Billing on probabilistic match is charging for money we cannot verify ourselves.

### C2 Attribution data must not create a conflict of interest

The platform fee exists so we do not earn more by inflating numbers. Requirements: versioned rules (`policy`), public to customers (`docs/product/attribution-policy-v1.md`), every record stores the deciding `policy_version` and an `evidence` snapshot.

`evidence` is append-only — it is the only evidence source for a KOL appeal.

### C3 Ledger is append-only

Refunds, chargebacks, and post-hoc fraud findings write reverse entries; never UPDATE the original. The DB revokes UPDATE/DELETE on `ledger_entry` for the app role.

`attribution.is_billable` is redundant (derivable from `method`) but must be stored — billing rules change; the basis of an issued invoice must stay frozen.

### C4 Capability gating is data-driven

No scattered `if plan == "pro"`. Drive from `plan_entitlement` + `tenant_entitlement_override`.

The override table is not optional: early sales will promise “Discord free for now”; without the table that promise becomes hard-coded, and in three months nobody can say what the customer bought.

---

## Design notes

### Redeem is the only stitch point on the full path

`apps/api/src/attribution/redeem.rs` is the most critical code in the system. Telegram identity (`tg_user_id` from initData) and app identity (`app_user_id` from the main app) bind here; attribution and the billable event are created together.

It must be one transaction: if the bind succeeds and attribution write fails, that user can never be attributed correctly — the claim code is spent, with no second chance.

`SELECT ... FOR UPDATE` is mandatory. Rapid taps or client retries race; without the lock both transactions can read `issued` and each write attribution + billing.

### Why CPA is based on redeem, not customer postback

MVP billable `external_id` is `claim:<id>` — redeem is a fact we can confirm.

If we billed on customer IAP postbacks, a missed postback is money we never collect and almost never detect. Pure take-rate on IAP has that structural weakness, so MVP charges on events we can confirm.

### Risk on redeem prefers hold over deny

Killing a real user means they get no prize and their first impression of the customer app is “this promo is a scam” — irreversible. Letting a farmer through is a temporary over-count; within the cooling window we can reverse or reject by hand before money leaves.

The only hard deny is device-dimension farming — a real user almost never binds three accounts on one device.

### Over-cap does not stop service

Over-cap conversions still attribute and still credit the KOL; they are simply not billed, marked “over cap (free)” on dashboards. Better UX than “stop after cap”, and a natural upsell narrative.

### Claim-code alphabet excludes 0/O/1/I/L

Manual entry is the only billable iOS path; one misread character is lost revenue.

`normalize_claim_code` deliberately does **not** map confusable characters: guessing `0` → `O` vs `D` is unreliable, and a wrong guess turns a valid code into another valid code under the wrong KOL. Prefer a format error and a re-type.

---

## Layout

```
Makefile / README.md              Orchestration & entry docs
configs/                          Runtime config (read from repo root)
db/
  migrations/                     Schema migrations
  seed.sql                        Demo data
docker/
  compose.yml                     Local Postgres + Redis
docs/
  product/                        Public contracts (attribution policy, …)
  design/                         Target system design
  engineering/                    Engineering notes (monorepo, …)
apps/                             Language workspaces (Cargo + pnpm live here)
  Cargo.toml / Cargo.lock
  package.json / pnpm-workspace.yaml / pnpm-lock.yaml
  api/                            Rust HTTP service / jobs / key tooling
  tma/                            Telegram Mini App
  packages/                       Shared JS/TS libs (second consumer only)
```

Cargo and pnpm manifests sit under `apps/`, not the repository root. Use `make …`
from the root (it passes `--manifest-path` on cargo subcommands and `pnpm --dir apps`).

API module detail: [apps/api/README.md](apps/api/README.md).

## Tech choices

Rust + axum + sqlx + tokio.

**sqlx enables `derive`, not `macros`:** `query!` needs a live Postgres at compile time, which makes CI and a newcomer’s first `cargo build` depend on a running DB. The trade-off is losing compile-time SQL checks for builds with no external dependency.

**The type system carries two core invariants:**

- `AttributionMethod::is_billable()` is an exhaustive `match` — adding a method forces a billability decision; forgetting cannot silently default.
- `ledger::Txn` fields are private and only constructible via `try_new` — **an unbalanced transaction cannot exist at the type level**.

`Cents` is a newtype, not a bare `i64`: the codebase mixes quantities, IDs, and durations as i64; mixing them is an easy, hard-to-spot class of bug.

Redis is in `docker/compose.yml` but unused in code yet — idempotency and rate limits currently rely on DB unique constraints, enough for MVP.

`#![allow(dead_code)]` at the top of `main.rs` is time-boxed: ledger, cap, and postback signing exist and are tested but are not all wired into the online path yet. Remove the allow once they are, so `dead_code` is a useful signal again.
