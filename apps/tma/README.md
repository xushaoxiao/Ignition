# Ignition TMA

Telegram Mini App game frontend. React + Vite + Tailwind. Package name `@ignition/tma`.

Ships two game shapes:

- five **prize-draw skins** — wheel, scratch card, slot machine, blind box, flip cards — all
  over the **same** server-authoritative outcome. See [Games](#games) below.
- one **daily decision game** — `daily_budget`, 每日理财决策. See
  [Daily budget game](#daily-budget-game).

`App.tsx` picks between them from the session's `game` field; both end at the same claim code.

This layer is the first instance of the `ChannelAdapter` extension point — use it to
validate whether the abstraction holds, rather than writing the abstraction first and
implementing later. When Discord Activities is wired up, what gets swapped is
[src/telegram.ts](src/telegram.ts), not the wheel.

## Two rules that must not change

### Outcomes come from the server; the frontend only plays the animation

`POST /v1/tma/play` returns `segment_index`; the frontend **derives** the angle to
land on from that. Do not spin first and ask for the result afterwards. Reversing that
order puts win probability in the client — prize-pool cost and the downstream billable
conversion both become untrustworthy.

### initData must be uploaded verbatim

The signature is computed over the raw field sequence. If the frontend parses and
re-serialises, even a slight mismatch in key order or escaping makes server-side
verification fail — showing up as “some users cannot open the app”, which is extremely
hard to debug.

## Games

A "game" is only an **animation skin over a server-decided outcome**. `POST /v1/tma/play`
returns a winning prize index (`segment_index`); every skin animates toward that index and
then calls `onSettled`. No skin ever picks the winner — that is the first rule above.

- **Contract** — [`components/games/types.ts`](src/components/games/types.ts): every game is a
  component taking `{ segments, target, spinning, onSettled }`.
- **Registry** — [`components/games/index.ts`](src/components/games/index.ts): maps the campaign's
  `template.code` (from the session's `game` field) to a component. Unknown codes fall back to the
  wheel, so a new backend template never breaks an older client.
- **Skins** — `Wheel`, `ScratchCard`, `SlotMachine`, `BlindBox`, `FlipCards`. The reveal-style
  ones share [`useReveal`](src/components/games/useReveal.ts) for the `idle → playing → revealed`
  lifecycle.

**Add a game**: add a `template` row (see `db/migrations/0004_game_templates.sql`), write a
component that implements `GameProps`, and register it by code in `games/index.ts`. Nothing in
the play flow, billing, or attribution changes.

**Preview without a backend**: `pnpm dev`, then open [`/?preview`](http://localhost:5173/?preview)
for a gallery that renders every game with mock prizes and a play button. To preview one skin
against a real campaign, append `?game=<code>` (cosmetic only — the outcome still comes from the
server; `?game=daily_budget` switches to the decision game against whatever campaign the session
opened).

## Daily budget game

`daily_budget` is **not** a skin, and deliberately does not implement `GameProps`: the player
makes a scored decision, so there is no prize index to animate toward. Forcing it into that
contract would hand every wheel-style skin props it does not use.

```text
今日场景 → 选择 → 评分 + 科普 → 排行榜 → 抽奖 → 兑换码
```

- [`daily/DailyApp.tsx`](src/daily/DailyApp.tsx) — the flow. Screens live beside it
  (`CreditMeter`, `ScenarioCard`, `OutcomeCard`, `Leaderboard`).
- [`PrizeFlow.tsx`](src/PrizeFlow.tsx) — the prize-draw half, shared with wheel-style campaigns.
  The daily game hands off to it with an explicit reward skin (`blind_box`).
- Endpoints: `GET /v1/tma/daily`, `POST /v1/tma/daily/answer`, `GET /v1/tma/daily/leaderboard`.

Three rules for this game, in addition to the two above:

1. **Scores never reach the client before the answer.** The server sends `key + label` only —
   same rule as the prize pool, which never sends weight or stock. A client that can read the
   score table turns the game into a lookup exercise and the leaderboard into copy-paste.
2. **The handoff to the draw is a UI sequence, not an entitlement.** Answering does not grant a
   play; `daily_play_limit` still decides how many draws a player gets. A score the player can
   influence must never unlock prize spend — see constraint C1 in the root README.
3. **The disclaimer under the tip stays.** Every tip is generic financial literacy, not advice
   about anyone's situation, and the soft promo above the score threshold is the customer's own
   claim from `campaign.config.promo` — never the game's voice.

To run it locally, point `DEV_TRACKING_ID` at a link whose campaign uses the template — the demo
seed ships one (`dQ7wN2pR5t`, campaign 2).

## The claim-code screen deserves repeated polish

There is no reliable user-level deferred deep link on iOS, so “the user manually enters
this code in the main app” is the **only** billable attribution path on that side. If
the code is hard to read, cannot be copied, or the next step is unclear, each issue is
direct revenue loss, not a cosmetic flaw.

The details in [src/components/ClaimCard.tsx](src/components/ClaimCard.tsx) that look
excessive elsewhere (monospace grouping at large size, explicit copy feedback, platform-
specific guidance) all follow from this. The core metric for the W7 seed beta is **iOS
claim-code redemption completion rate > 40%**.

## Local development

Install dependencies and start from the **repository root** (pnpm workspace lives under `apps/`):

```bash
# from repo root
make tma-install
cp apps/tma/.env.example apps/tma/.env.local
make tma-dev
# or: pnpm --dir apps --filter @ignition/tma dev
```

Telegram only loads HTTPS pages; real-device debugging needs a tunnel:

```bash
cloudflared tunnel --url http://localhost:5173
# paste the https URL into @BotFather Mini App settings
```

### Run the full flow in a browser without a tunnel

`pnpm dev` exposes a `/__dev/init-data` endpoint that signs initData on the fly with
the bot token; the frontend falls back to it outside Telegram. **Every request is
fresh** — initData expires after 5 minutes; a pre-signed string in `.env` starts
rotting as soon as it is signed.

Configure two values in `apps/tma/.env.local`:

```bash
DEV_BOT_TOKEN=123456:AA-demo-bot-token   # must match bot.token_enc in the database
DEV_TRACKING_ID=aB3xY9zK1m
```

**Omitting the `VITE_` prefix is deliberate**: those variables are only readable on the
Node side in `vite.config.ts` and never enter any frontend bundle. Signing happens
entirely in the dev server; `apply: 'serve'` means `vite build` never ships this code.

The backend must also allow the local origin in `configs/config.yaml` at the repo root:

```yaml
http:
  cors_origins: ["http://localhost:5173"]
```

Use an allow list, not `*`: these endpoints carry Bearer tokens.

## Build

```bash
# from repo root
make tma-build
# or: pnpm --dir apps --filter @ignition/tma build
```

Output is static files; deploy `apps/tma/dist/` to a CDN. In production, reverse-proxy
the API to the same origin to avoid CORS.
