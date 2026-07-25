# @ignition/landing

Public marketing site for Ignition. Static (SSG), bilingual (`/zh`, `/en`).

## Stack

- **Next.js 16** — App Router, prerendered to static HTML
- **HeroUI v3** — React component library (React Aria based). v3 needs **no**
  provider; components read theme from CSS variables
- **Tailwind CSS v4** — CSS-first config (`app/globals.css`, no `tailwind.config`)
- **next-themes** — light/dark via a `.dark` class on `<html>`

## Run

From the repo root:

```bash
make tma-install    # shared workspace install (pnpm --dir apps install)
make landing-dev    # dev server, http://localhost:3000
make landing-build  # production build (both locales prerender)
```

Or with pnpm directly: `pnpm --dir apps --filter @ignition/landing <dev|build|typecheck>`.

## Layout

```
app/
  [locale]/            Root layout (<html lang>) + page; the only route tree
  providers.tsx        next-themes provider (theme only)
  globals.css          Tailwind + HeroUI imports; brand accent override
proxy.ts               Redirects "/" and unprefixed paths to a locale (Next 16
                       renamed the "middleware" convention to "proxy")
components/
  section.tsx          Shared Section / SectionHeading primitives
  site-header.tsx      Sticky nav + language & theme switches (client)
  sections/            Hero, positioning, features, how-it-works, trust,
                       pricing, faq (HeroUI Accordion), cta
i18n/
  config.ts            locales, defaultLocale, isLocale()
  dictionaries.ts      getDictionary(locale)
  dictionaries/en.ts   English copy — the source-of-truth shape
  dictionaries/zh.ts   Chinese copy — typed `typeof en`, so the two stay in sync
```

## Deploy (Vercel)

The pnpm workspace root is `apps/`, so Vercel needs the app as its root:

1. **vercel.com → Add New → Project**, import `xushaoxiao/Ignition`.
2. **Root Directory** → `apps/landing` (Edit → select the folder). Keep
   **"Include files outside of the root directory"** enabled so the workspace
   `pnpm-lock.yaml` is visible.
3. Framework (**Next.js**), install (`pnpm install --frozen-lockfile`) and build
   (`next build`) are picked up from [vercel.json](./vercel.json) / auto-detect —
   no manual overrides needed.
4. Deploy. Every push to `main` then ships automatically.

`/` redirects to a locale by `Accept-Language` (see [proxy.ts](./proxy.ts)); set a
real domain and update `metadataBase` in `app/[locale]/layout.tsx`.

## Conventions

- **Copy lives in dictionaries only.** Add a key to `en.ts`; TypeScript then
  forces the same key into `zh.ts`. Sections receive a typed slice as `dict`.
- **Brand colour is one source.** `app/globals.css` overrides HeroUI's `--accent`
  to the Ignition red-orange, and the Tailwind `brand` utility resolves to the
  same variable — so HeroUI components and custom sections stay in lockstep
  across light/dark. Change it in one place.
- **Add a locale**: extend `locales` in `i18n/config.ts` and add a dictionary;
  `generateStaticParams` and `proxy.ts` pick it up automatically.
