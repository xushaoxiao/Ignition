import type { FC } from 'react'

/** One prize slot. Structurally matches the TMA session's prize shape. */
export interface Segment {
  id: number
  label: string
}

/**
 * The contract every game skin implements.
 *
 * A game is **only an animation over a server-decided outcome**. `target` is the winning prize
 * index the server returned (`segment_index`); the component animates toward it and calls
 * `onSettled` when the reveal finishes. No game ever decides the winner — that would move win
 * probability into the client and make prize cost and the billable conversion untrustworthy.
 */
export interface GameProps {
  /** Prize pool in server order; `target` indexes into this. */
  segments: Segment[]
  /** Winning prize index; `null` while idle. */
  target: number | null
  /** True while the reveal animation should run. */
  spinning: boolean
  /** Called once when the reveal animation has settled. */
  onSettled: () => void
}

export type GameComponent = FC<GameProps>
