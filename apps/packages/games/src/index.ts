/**
 * Game registry.
 *
 * Maps a campaign's `template.code` (from the session) to an animation skin. Every skin obeys the
 * same {@link GameProps} contract — the app's play flow (`play → reveal → claim`) is identical
 * regardless of which game is shown. An unknown code falls back to the wheel, so a new backend
 * template never breaks an older client.
 */
import { BlindBox } from './BlindBox'
import { FlipCards } from './FlipCards'
import { ScratchCard } from './ScratchCard'
import { SlotMachine } from './SlotMachine'
import { Wheel } from './Wheel'
import type { GameComponent } from './types'

export type { GameProps, GameComponent, Segment } from './types'

interface GameMeta {
  title: string
  Component: GameComponent
}

const FALLBACK: GameMeta = { title: '幸运大转盘', Component: Wheel }

const REGISTRY: Record<string, GameMeta> = {
  lucky_wheel: FALLBACK,
  scratch_card: { title: '刮刮卡', Component: ScratchCard },
  slot_machine: { title: '幸运老虎机', Component: SlotMachine },
  blind_box: { title: '幸运盲盒', Component: BlindBox },
  flip_card: { title: '翻牌有礼', Component: FlipCards },
}

/** Resolve a game by template code, falling back to the wheel for unknown/missing codes. */
export function gameFor(code: string | undefined | null): GameMeta {
  return (code ? REGISTRY[code] : undefined) ?? FALLBACK
}

/** All registered template codes — used by the dev preview gallery. */
export const GAME_CODES = Object.keys(REGISTRY)
