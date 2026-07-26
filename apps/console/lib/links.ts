import type { Campaign } from "./types";

// The bot the Mini App is attached to. Configurable per deploy; a placeholder here since this is
// the frontend-first mock. The real console reads it from the tenant's bot config.
const BOT = process.env.NEXT_PUBLIC_TMA_BOT ?? "ignition_demo_bot";
// Where the TMA is hosted, for the "preview in Mini App" convenience link.
const TMA_BASE = process.env.NEXT_PUBLIC_TMA_BASE ?? "http://localhost:5273";

/** Telegram deep link a KOL/channel distributes — start_param carries the tracking id. */
export function deepLink(c: Campaign): string {
  return `https://t.me/${BOT}/app?startapp=${c.trackingId}`;
}

/** Convenience link to preview the chosen game skin in the running TMA (cosmetic only). */
export function previewLink(c: Campaign): string {
  return `${TMA_BASE}/?game=${encodeURIComponent(c.game)}`;
}
