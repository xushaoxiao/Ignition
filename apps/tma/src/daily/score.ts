/**
 * Display-only constants for the credit meter.
 *
 * These mirror `CREDIT_MIN` / `CREDIT_MAX` in `apps/api/src/daily/mod.rs` and are used **only** to
 * draw the bar. Scores themselves always arrive from the server: nothing on the client adds,
 * clamps, or predicts a score, so a copy drifting here can never change what a player is awarded —
 * at worst the bar is drawn against the wrong end points.
 */
export const CREDIT_MIN = 300
export const CREDIT_MAX = 850

/** Format a score change as a signed chip label ("+12" / "-8"). */
export function signed(n: number): string {
  return n > 0 ? `+${n}` : `${n}`
}
