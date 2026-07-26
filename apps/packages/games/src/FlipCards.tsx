/**
 * Flip-card skin.
 *
 * Three face-down cards flip over; the centre card reveals `segments[target]` (the server outcome),
 * the others show decoys. Which card wins is fixed — the flip is just presentation.
 */
import type { GameProps } from './types'
import { symbolFor, truncate } from './shared'
import { useReveal } from './useReveal'

const REVEAL_AT = 300
const SETTLE_AT = 1500
const WINNER = 1 // centre card

export function FlipCards({ segments, target, spinning, onSettled }: GameProps) {
  const stage = useReveal(spinning, target, onSettled, REVEAL_AT, SETTLE_AT)
  const flipped = stage !== 'idle'
  const won = target !== null ? segments[target] : null

  return (
    <div className="mx-auto w-full max-w-[20rem] select-none">
      <div className="flex justify-center gap-3">
        {[0, 1, 2].map((k) => {
          const winner = k === WINNER
          return (
            <div key={k} style={{ perspective: '800px' }} className="h-32 w-[5.5rem]">
              <div
                className={`flip-inner h-full w-full ${flipped ? 'is-flipped' : ''}`}
                style={{ transitionDelay: `${k * 140}ms` }}
              >
                <div className="flip-face bg-white/10 text-3xl ring-1 ring-white/15">❓</div>
                <div
                  className={`flip-face flip-back px-1 text-center ring-1 ${
                    winner
                      ? 'bg-gradient-to-br from-amber-400/40 to-fuchsia-500/40 ring-amber-300/40'
                      : 'bg-white/5 ring-white/10'
                  }`}
                >
                  {winner && won ? (
                    <div>
                      <div className="text-3xl">{symbolFor(target!)}</div>
                      <div className="mt-1 text-[0.7rem] font-semibold leading-tight">
                        {truncate(won.label, 10)}
                      </div>
                    </div>
                  ) : (
                    <div className="text-3xl opacity-50">{symbolFor(k + 3)}</div>
                  )}
                </div>
              </div>
            </div>
          )
        })}
      </div>
      <p className="mt-4 text-center text-sm text-white/60">
        {flipped ? (stage === 'revealed' ? '翻牌成功' : '翻牌中…') : '点击翻牌'}
      </p>
    </div>
  )
}
