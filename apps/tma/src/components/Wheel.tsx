/**
 * Wheel.
 *
 * **Animation is theatre only.** The outcome is fixed on the server; after receiving
 * `segment_index`, the frontend derives the landing angle — not spin first, then ask.
 * Reversing that order puts win probability on the client; prize-pool cost and the
 * downstream billable conversion become untrustworthy.
 *
 * CSS transform + cubic-bezier instead of a game engine: one wheel does not justify
 * hundreds of KB in the bundle, and transform composes on the GPU — more reliable on
 * low-end Android.
 */
import { useEffect, useRef, useState } from 'react'

export interface Segment {
  id: number
  label: string
}

interface Props {
  segments: Segment[]
  /** Target segment index; null when idle. */
  target: number | null
  spinning: boolean
  onSettled: () => void
}

/** Full turns before stop. Fewer than 3 feels like stutter; more than 6 feels slow. */
const FULL_TURNS = 5
const SPIN_MS = 4200

const PALETTE = [
  '#6366f1', '#ec4899', '#f59e0b', '#10b981',
  '#3b82f6', '#a855f7', '#ef4444', '#14b8a6',
]

export function Wheel({ segments, target, spinning, onSettled }: Props) {
  const [rotation, setRotation] = useState(0)
  // Cumulative turns only increase: each spin continues from the current angle; the pointer never jumps backwards.
  const turns = useRef(0)

  useEffect(() => {
    if (!spinning || target === null || segments.length === 0) return

    const step = 360 / segments.length
    // Pointer fixed at 12 o'clock. To centre segment `target` under it, rotate the
    // disc backwards by that segment's centre angle.
    const center = target * step + step / 2
    turns.current += FULL_TURNS
    setRotation(turns.current * 360 - center)

    const t = window.setTimeout(onSettled, SPIN_MS)
    return () => window.clearTimeout(t)
  }, [spinning, target, segments.length, onSettled])

  const step = segments.length > 0 ? 360 / segments.length : 360

  return (
    <div className="relative mx-auto aspect-square w-full max-w-[20rem]">
      {/* Pointer */}
      <div className="absolute left-1/2 top-0 z-10 -translate-x-1/2 -translate-y-1">
        <div className="h-0 w-0 border-x-[0.75rem] border-t-[1.25rem] border-x-transparent border-t-amber-300 drop-shadow" />
      </div>

      <div
        className="h-full w-full rounded-full ring-4 ring-white/15"
        style={{
          transform: `rotate(${rotation}deg)`,
          transition: spinning
            ? // Fast start, very slow tail: suspense in the last few degrees is the whole point
              `transform ${SPIN_MS}ms cubic-bezier(0.12, 0.7, 0.05, 1)`
            : 'none',
        }}
      >
        <svg viewBox="-50 -50 100 100" className="h-full w-full">
          {segments.map((seg, i) => (
            <g key={seg.id}>
              <path d={sector(i * step, step)} fill={PALETTE[i % PALETTE.length]} />
              <text
                x={0}
                y={0}
                transform={labelTransform(i * step + step / 2)}
                textAnchor="middle"
                dominantBaseline="middle"
                fill="white"
                fontSize="5.5"
                fontWeight="600"
              >
                {truncate(seg.label)}
              </text>
            </g>
          ))}
          <circle r="7" fill="#fff" opacity="0.9" />
        </svg>
      </div>
    </div>
  )
}

/**
 * Segment label position and orientation.
 *
 * Text runs radially. Lower-half segments need an extra 180° flip or labels read upside-down —
 * users cannot tell what they are spinning for.
 */
function labelTransform(centerDeg: number): string {
  const flip = centerDeg > 90 && centerDeg < 270
  return `rotate(${centerDeg}) translate(0 -30) rotate(${flip ? 180 : 0})`
}

/** Sector path from centre, clockwise from 12 o'clock. */
function sector(startDeg: number, sweepDeg: number): string {
  const r = 50
  const a0 = ((startDeg - 90) * Math.PI) / 180
  const a1 = ((startDeg + sweepDeg - 90) * Math.PI) / 180
  const x0 = r * Math.cos(a0)
  const y0 = r * Math.sin(a0)
  const x1 = r * Math.cos(a1)
  const y1 = r * Math.sin(a1)
  const largeArc = sweepDeg > 180 ? 1 : 0
  return `M 0 0 L ${x0} ${y0} A ${r} ${r} 0 ${largeArc} 1 ${x1} ${y1} Z`
}

/** Truncate long prize names in tight segments; full name on the result screen. */
function truncate(s: string): string {
  return s.length > 7 ? s.slice(0, 6) + '…' : s
}
