/**
 * Virtual credit score header.
 *
 * The number is the whole retention loop: it is the thing that survives between sessions, so it
 * gets the top of the screen, the streak next to it, and the rank underneath. All three values
 * come from the server — the client never adds up a score locally, or two devices would disagree
 * about the same player.
 */
import type { Grade } from '../api'
import { CREDIT_MAX, CREDIT_MIN } from './score'

interface Props {
  credit: number
  grade: Grade
  gradeLabel: string
  streak: number
  rank: number | null
  players: number
}

const TONE: Record<Grade, { bar: string; text: string }> = {
  building: { bar: 'bg-rose-400', text: 'text-rose-300' },
  steady: { bar: 'bg-amber-400', text: 'text-amber-300' },
  strong: { bar: 'bg-emerald-400', text: 'text-emerald-300' },
  excellent: { bar: 'bg-sky-400', text: 'text-sky-300' },
}

export function CreditMeter({ credit, grade, gradeLabel, streak, rank, players }: Props) {
  const tone = TONE[grade] ?? TONE.steady
  const pct = Math.round(((credit - CREDIT_MIN) / (CREDIT_MAX - CREDIT_MIN)) * 100)

  return (
    <section className="rounded-2xl bg-white/8 p-4 ring-1 ring-white/12">
      <div className="flex items-end justify-between">
        <div>
          <p className="text-xs text-white/55">我的理财分</p>
          <p className="mt-0.5 flex items-baseline gap-2">
            {/* Tabular figures: the number changes by a few points at a time and should not
                make the row jump. */}
            <span className="font-mono text-4xl font-bold tabular-nums">{credit}</span>
            <span className={`text-sm font-semibold ${tone.text}`}>{gradeLabel}</span>
          </p>
        </div>
        <div className="text-right">
          <p className="text-lg font-bold">🔥 {streak} 天</p>
          <p className="text-xs text-white/55">连续打卡</p>
        </div>
      </div>

      <div className="mt-3 h-2 overflow-hidden rounded-full bg-white/10">
        <div
          className={`h-full rounded-full transition-[width] duration-700 ease-out ${tone.bar}`}
          style={{ width: `${Math.max(2, Math.min(100, pct))}%` }}
        />
      </div>
      <div className="mt-1.5 flex justify-between text-[11px] text-white/40">
        <span>{CREDIT_MIN}</span>
        <span>
          {rank ? `第 ${rank} 名 / ${players} 人` : players > 0 ? `${players} 人已参与` : '今日首答即上榜'}
        </span>
        <span>{CREDIT_MAX}</span>
      </div>
    </section>
  )
}
