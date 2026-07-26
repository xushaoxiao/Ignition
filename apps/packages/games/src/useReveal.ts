import { useEffect, useState } from 'react'

export type RevealStage = 'idle' | 'playing' | 'revealed'

/**
 * Drives the shared reveal lifecycle for reveal-style games (scratch / slot / box / flip):
 *
 * ```text
 * idle ──spin──▶ playing ──revealAt──▶ revealed ──settleAt──▶ onSettled()
 * ```
 *
 * The prize shown when `revealed` is always `segments[target]` — the server's outcome. Timers are
 * cleaned up on change so a fast re-play never double-fires `onSettled`. When `target` clears
 * (the app resets for another play) the stage returns to `idle`.
 */
export function useReveal(
  spinning: boolean,
  target: number | null,
  onSettled: () => void,
  revealAt: number,
  settleAt: number,
): RevealStage {
  const [stage, setStage] = useState<RevealStage>('idle')

  useEffect(() => {
    if (target === null) {
      setStage('idle')
      return
    }
    if (!spinning) return // e.g. the claim phase after settle: keep the revealed prize on screen.

    setStage('playing')
    const reveal = window.setTimeout(() => setStage('revealed'), revealAt)
    const settle = window.setTimeout(onSettled, settleAt)
    return () => {
      window.clearTimeout(reveal)
      window.clearTimeout(settle)
    }
  }, [spinning, target, onSettled, revealAt, settleAt])

  return stage
}
