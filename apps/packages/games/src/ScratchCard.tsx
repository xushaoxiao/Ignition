/**
 * Scratch-card skin.
 *
 * The prize sits under a foil overlay. When the server outcome arrives the foil dissolves to
 * reveal `segments[target]` — the reveal is driven by the server result, never by how much the
 * user "scratches".
 */
import type { GameProps } from './types'
import { symbolFor, truncate } from './shared'
import { useReveal } from './useReveal'

const REVEAL_AT = 150
const SETTLE_AT = 1500

export function ScratchCard({ segments, target, spinning, onSettled }: GameProps) {
  const stage = useReveal(spinning, target, onSettled, REVEAL_AT, SETTLE_AT)
  const revealed = stage !== 'idle'
  const won = target !== null ? segments[target] : null

  return (
    <div className="relative mx-auto aspect-[7/4] w-full max-w-[20rem] select-none">
      {/* Prize underneath */}
      <div className="absolute inset-0 grid place-items-center rounded-3xl bg-gradient-to-br from-indigo-500/25 to-fuchsia-500/25 ring-1 ring-white/10">
        <div className="text-center">
          <div className="text-5xl">{won ? symbolFor(target!) : '🎁'}</div>
          <div className="mt-2 text-lg font-bold">{won ? truncate(won.label, 12) : '奖品'}</div>
        </div>
      </div>

      {/* Foil overlay */}
      <div
        className={`absolute inset-0 grid place-items-center overflow-hidden rounded-3xl bg-gradient-to-br from-slate-300 to-slate-500 transition-all duration-700 ${
          revealed ? 'scale-105 opacity-0' : 'opacity-100'
        }`}
      >
        <span className="z-10 text-sm font-semibold text-slate-800/80">
          {spinning ? '刮开中…' : '刮开查看奖品'}
        </span>
        <div className="game-shimmer" />
      </div>
    </div>
  )
}
