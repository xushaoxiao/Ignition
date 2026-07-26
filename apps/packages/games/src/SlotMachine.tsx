/**
 * Slot-machine skin.
 *
 * Three reels scroll and decelerate onto the same symbol — the one for `segments[target]`. The
 * reels are pure theatre: they always land on the server's winning prize, staggered so the last
 * reel "locks in" for suspense.
 */
import type { GameProps } from './types'
import { symbolFor, truncate } from './shared'
import { useReveal } from './useReveal'

const CELL_REM = 4.5
const STRIP = 18 // symbols per reel; the last one is the winner.
const REVEAL_AT = 2100 // last reel has locked by here.
const SETTLE_AT = 2400

/** A reel of decoy symbols ending on the winning symbol. Deterministic per reel for stable decoys. */
function reelStrip(reel: number, target: number): string[] {
  const cells: string[] = []
  for (let j = 0; j < STRIP - 1; j++) cells.push(symbolFor(reel * 4 + j * 3 + 2))
  cells.push(symbolFor(target))
  return cells
}

export function SlotMachine({ segments, target, spinning, onSettled }: GameProps) {
  const stage = useReveal(spinning, target, onSettled, REVEAL_AT, SETTLE_AT)
  const rolling = stage !== 'idle'
  const won = target !== null ? segments[target] : null
  const end = (STRIP - 1) * CELL_REM

  return (
    <div className="mx-auto w-full max-w-[20rem] select-none">
      <div className="flex justify-center gap-2 rounded-3xl bg-gradient-to-b from-amber-500/20 to-fuchsia-600/20 p-4 ring-1 ring-white/10">
        {[0, 1, 2].map((reel) => (
          <div
            key={reel}
            className="overflow-hidden rounded-2xl bg-black/45 ring-1 ring-white/15"
            style={{ height: `${CELL_REM}rem`, width: `${CELL_REM}rem` }}
          >
            <div
              style={{
                transform: rolling ? `translateY(-${end}rem)` : 'translateY(0)',
                // Longer per reel → they lock left-to-right.
                transition: rolling
                  ? `transform ${1200 + reel * 450}ms cubic-bezier(0.15, 0.7, 0.2, 1)`
                  : 'none',
              }}
            >
              {reelStrip(reel, target ?? 0).map((s, j) => (
                <div key={j} className="grid place-items-center" style={{ height: `${CELL_REM}rem` }}>
                  <span className="text-4xl">{s}</span>
                </div>
              ))}
            </div>
          </div>
        ))}
      </div>

      <p className="mt-4 text-center text-lg font-bold">
        {stage === 'revealed' && won ? `🎉 ${truncate(won.label, 12)}` : '拉动老虎机'}
      </p>
    </div>
  )
}
