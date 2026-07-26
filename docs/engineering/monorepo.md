# Monorepo layout

Ignition keeps **language workspaces under `apps/`**. The repository root only
orchestrates (Makefile, docs, configs, db, docker).

```
Makefile / README.md / CLAUDE.md   Orchestration & docs
configs/                           Runtime config (API reads from repo root cwd)
db/                                Migrations + seed
docker/                            Local Postgres + Redis
docs/                              Product / design / engineering

apps/                              ← Cargo + pnpm workspace root
  Cargo.toml / Cargo.lock          Rust workspace (member: api)
  package.json / pnpm-workspace.yaml / pnpm-lock.yaml
  api/                             Rust HTTP service, jobs, CLI (package: ignition)
  tma/                             Telegram Mini App (@ignition/tma)
  landing/                         Marketing site, Next.js + HeroUI (@ignition/landing)
  console/                         Campaign builder, Next.js + HeroUI (@ignition/console)
  packages/games/                  Shared game skins (@ignition/games) — TMA + console
```

## Why manifests live under `apps/`

The repo root stays free of language lockfiles. Cargo and pnpm each resolve their
workspace from `apps/`. The root `Makefile` points at them:

```makefile
MANIFEST = --manifest-path apps/Cargo.toml
PNPM     = pnpm --dir apps
# e.g. cargo test $(MANIFEST) -p ignition
```

Config paths (`configs/config.yaml`) stay relative to the **repository root**
because Make invokes cargo with the repo as cwd.

## Workspaces

- **Cargo** — `apps/Cargo.toml` members `["api"]`. From repo root: `make test` or
  `cargo test --manifest-path apps/Cargo.toml -p ignition`.
- **pnpm** — `apps/pnpm-workspace.yaml` includes `tma` and `packages/*`.
  Prefer `make tma-*` or `pnpm --dir apps --filter @ignition/tma …`.

## Local containers

```bash
make up
docker compose -f docker/compose.yml up -d --wait
```

## Adding an app

1. Put it under `apps/<name>/`.
2. Rust: add the path to `apps/Cargo.toml` `[workspace].members`.
3. JS/TS: name the package `@ignition/<name>`; list it in `apps/pnpm-workspace.yaml`.
4. Wire root `Makefile` targets; keep the root README quick-start in sync.

Do not extract shared crates/packages until a second app actually needs them.
