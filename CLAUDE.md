# Ignition engineering conventions

Read the “Four non-negotiable constraints” in `README.md` first. Many seemingly
roundabout choices only make sense against those four; changing code without
them is an easy way to break revenue correctness.

For more background (why modules are cut this way, why billing only accepts
deterministic attribution, milestone order), read `docs/design/system-design.md`.
It describes the **target** state; actual progress is in README. **If your change
drifts from the design doc, update the doc — do not let it rot.**

Repo layout: `docs/engineering/monorepo.md` (Cargo/pnpm workspaces under `apps/`).

## Before you change

- **Attribution or billing logic** → read `docs/product/attribution-policy-v1.md`
  first. That doc is customer-facing; code must match it. Rule changes need a new
  `policy_version`; do not change v1 semantics in place.
- **`apps/api/src/models.rs` attribution / state-machine mapping** → the tests are
  deliberate guards. Do not retarget expectations to green-light new logic.
- **`redeem.rs`** → sole stitch point of the full path; keep the whole flow in one
  transaction; do not remove `SELECT ... FOR UPDATE`.
- **S2S request parsing** → handlers must verify the signature on the raw `Bytes`
  before deserialising. Switching to a `Json<T>` extractor breaks signatures and
  shows up as “random 401s for some customers”.
- **`jobs/settle.rs`** → the only place ledger, cap, and state machine enter the
  online path together. Select unsettled events with `invoice_id IS NULL`, **not**
  by billing-period window — window filters permanently drop late-cleared events.

## Hard rules

1. **Amounts are always `Cents`** — no floats, and do not regress to bare `i64`.
2. **Ledger is append-only** — never `UPDATE ledger_entry`; reverse with
   `Txn::reverse`. The DB already revoked UPDATE/DELETE for the app role.
3. **Status changes go through `billing::transition`** — do not assign `ev.status`
   directly.
4. **Tenant data goes through `db::begin_tenant_tx`** — do not query tenant tables
   on the raw `pool`; that skips RLS context and returns empty (fail-closed).
5. **New attribution methods: take the `is_billable` match seriously** — the
   compiler forces a branch, but a wrong branch is unchecked. Default should be
   non-billable.
6. **Secrets never enter logs.** Decrypt `_enc` fields into `secrets::Secret`
   only; its `Debug` is always `<redacted>`. Plaintext requires an explicit
   `expose()` — a reviewable call site. Do not unwrap into `String` for logging.
7. **New capability switches go through entitlement** — no `if plan == "pro"`.
8. **Tenant identity only from `auth::Caller` or `jwt::Claims`** — never from the
   body, query string, or custom headers (that was the placeholder API-Key signing
   replaced).
9. **Postgres `sum(bigint)` returns `NUMERIC`** — cast aggregates to `::bigint`
   explicitly or sqlx decode fails. Hit in settle and ledger audit already.

## Frontend

TMA lives in `apps/tma/`; conventions in [apps/tma/README.md](./apps/tma/README.md).
Two rules that must not change: play outcomes are produced by the server (the
client only animates); initData must be uploaded verbatim (signatures cover the
raw field serialisation).

## Tests

- `cargo test -p ignition` (via `apps/Cargo.toml` / `make test`) is pure unit tests with no database and must always
  pass. Keep tests in the same file (`#[cfg(test)] mod tests`), per Rust convention.
- Avoid sleep: inject time (`now: DateTime<Utc>`); do not read the wall clock.
  `telegram::verify` and `hmacsig::verify` take `now` for that reason.

## Commits

Commit messages in English.
