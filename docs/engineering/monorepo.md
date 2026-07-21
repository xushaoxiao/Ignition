# Monorepo layout

Ignition is a Cargo + pnpm monorepo.

```
apps/
  api/          Rust HTTP service, jobs, CLI (package name: ignition)
  tma/          Telegram Mini App (@ignition/tma)
packages/       Shared libraries — add only when a second consumer exists
db/
  migrations/   Schema migrations
  seed.sql      Local demo data
configs/        Runtime config (read from repo root)
docs/
  product/      Public contracts
  design/       Target architecture
  engineering/  How we work in this repo
```

## Workspaces

- **Cargo** — root `Cargo.toml` lists `apps/api`. Run from repo root: `cargo test -p ignition`.
- **pnpm** — root `pnpm-workspace.yaml` includes `apps/*` and `packages/*`. Prefer `pnpm --filter @ignition/tma …` or `make tma-*`.

## Adding an app

1. Put it under `apps/<name>/`.
2. Rust: add the path to `[workspace].members`.
3. JS/TS: ensure `package.json` name is `@ignition/<name>`; pnpm picks it up via `apps/*`.
4. Wire root `Makefile` targets; keep the root README quick-start in sync.

Do not extract shared crates/packages until a second app actually needs them.
