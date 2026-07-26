/**
 * Blind-box skin.
 *
 * The box shakes, then bursts open to reveal `segments[target]`. What is inside is the server's
 * outcome; the shake is just anticipation.
 */
import type { GameProps } from './types'
import { symbolFor, truncate } from './shared'
import { useReveal } from './useReveal'

const OPEN_AT = 900
const SETTLE_AT = 1800

export function BlindBox({ segments, target, spinning, onSettled }: GameProps) {
  const stage = useReveal(spinning, target, onSettled, OPEN_AT, SETTLE_AT)
  const won = target !== null ? segments[target] : null

  return (
    <div className="relative mx-auto grid aspect-square w-full max-w-[16rem] place-items-center">
      {stage === 'revealed' && won ? (
        <div className="game-pop text-center">
          <div className="text-8xl drop-shadow">{symbolFor(target!)}</div>
          <div className="mt-3 text-lg font-bold">{truncate(won.label, 12)}</div>
        </div>
      ) : (
        <div className="text-center">
          <div className={`text-8xl drop-shadow ${stage === 'playing' ? 'game-shake' : ''}`}>🎁</div>
          <div className="mt-3 text-sm text-white/60">
            {stage === 'playing' ? '开启中…' : '神秘盲盒'}
          </div>
        </div>
      )}
    </div>
  )
}
