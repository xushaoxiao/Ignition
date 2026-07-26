# @ignition/console

Self-serve **campaign builder** for a customer's marketing team. They configure a
gamified activity and generate a ready-to-distribute page — no engineering involved.

Next.js 16 (App Router) + HeroUI v3 + Tailwind v4. Package name `@ignition/console`.

## Status: frontend-first

This iteration is the **UI wired to an in-browser mock API** ([`lib/mockApi.ts`](lib/mockApi.ts),
localStorage). It exists to shape the whole flow before the backend lands. The next step is
the real backend:

- **Tenant-admin auth** — marketing staff sign in with provisioned accounts (email + password,
  stateful console session). Does not exist yet (today's callers are the S2S API key and the
  end-user TMA JWT).
- **Config APIs** — create/update campaign + prizes + link, with `config_schema` validation and
  RLS tenant context. `lib/mockApi.ts` mirrors the shape, so swapping to `fetch()` is one file.

## The flow

1. **活动基础** — name, daily play limit (risk L1), optional start/end.
2. **选择玩法** — pick a game; the [`GamePreview`](components/GamePreview.tsx) renders the **real**
   skin from [`@ignition/games`](../packages/games) with the configured prizes.
3. **配置奖池** — edit prizes (label · weight · stock); win-rates update live.
4. **生成** — publish creates a campaign with a confusable-free tracking id and returns a Telegram
   deep link + QR (the "generated page" — the TMA renders the activity from the campaign config).

## Run

```bash
make tma-install     # shared workspace install (pnpm --dir apps install)
make console-dev     # http://localhost:3000
make console-build
```

Env (optional): `NEXT_PUBLIC_TMA_BOT` (bot the Mini App is attached to) and
`NEXT_PUBLIC_TMA_BASE` (TMA origin for the "preview in Mini App" link).

## Notes

- **Games are not duplicated** — the preview uses `@ignition/games`, the same components the TMA
  ships. Next transpiles the workspace package (`transpilePackages` in `next.config.mjs`) and
  Tailwind scans it via `@source` in `app/globals.css`.
- The preview outcome is a local weighted pick for realism; the **real** outcome is always
  server-side (that rule lives in the game skins and the API, never here).
