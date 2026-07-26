/** Visual helpers shared across game skins. Purely cosmetic — never affect the outcome. */

/** Segment fill colours (wheel sectors, card accents). */
export const PALETTE = [
  '#6366f1', '#ec4899', '#f59e0b', '#10b981',
  '#3b82f6', '#a855f7', '#ef4444', '#14b8a6',
]

/** A decorative emoji per prize slot — gives slots/reels/boxes visual identity without needing
 *  per-prize art from the tenant. Stable per index so the same prize always shows the same face. */
const SYMBOLS = ['🎁', '💎', '⭐', '🍒', '🔔', '🍀', '🪙', '👑', '🍋', '7️⃣']

function wrap(i: number, len: number): number {
  return ((i % len) + len) % len
}

export function symbolFor(i: number): string {
  return SYMBOLS[wrap(i, SYMBOLS.length)] ?? '🎁'
}

export function colorFor(i: number): string {
  return PALETTE[wrap(i, PALETTE.length)] ?? '#6366f1'
}

/** Truncate long prize names in tight spaces; the full name shows on the claim screen. */
export function truncate(s: string, n = 8): string {
  return s.length > n ? s.slice(0, n - 1) + '…' : s
}
