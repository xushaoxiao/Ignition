/**
 * Campaign leaderboard.
 *
 * Collapsed to the top few by default: the question the player actually has is "am I near the
 * top", and a full list pushes the prize-draw button below the fold — that button is the only
 * step on this screen that leads to revenue.
 */
import { useState } from 'react'
import type { DailyLeaderboard } from '../api'

const PREVIEW = 5

interface Props {
  board: DailyLeaderboard | null
}

export function Leaderboard({ board }: Props) {
  const [expanded, setExpanded] = useState(false)

  if (!board || board.entries.length === 0) return null
  const shown = expanded ? board.entries : board.entries.slice(0, PREVIEW)
  const mineOffBoard =
    board.my_rank !== null && !board.entries.some((e) => e.me) ? board.my_rank : null

  return (
    <section className="rounded-2xl bg-white/8 p-4 ring-1 ring-white/12">
      <div className="flex items-baseline justify-between">
        <h3 className="text-sm font-semibold">🏆 理财分排行榜</h3>
        <span className="text-xs text-white/45">{board.players} 人参与</span>
      </div>

      <ol className="mt-3 flex flex-col gap-1.5">
        {shown.map((e) => (
          <li
            key={`${e.rank}-${e.name}`}
            className={`flex items-center gap-3 rounded-xl px-3 py-2 text-sm ${
              e.me ? 'bg-amber-400/15 ring-1 ring-amber-300/30' : 'bg-white/5'
            }`}
          >
            <span className="w-6 shrink-0 font-mono text-white/50 tabular-nums">{e.rank}</span>
            <span className="min-w-0 flex-1 truncate">
              {e.name}
              {e.me && <span className="ml-1 text-xs text-amber-300">（我）</span>}
            </span>
            <span className="shrink-0 text-xs text-white/45">🔥{e.streak}</span>
            <span className="w-10 shrink-0 text-right font-mono font-semibold tabular-nums">
              {e.credit}
            </span>
          </li>
        ))}
      </ol>

      {mineOffBoard && (
        <p className="mt-2 text-center text-xs text-white/50">
          我当前第 {mineOffBoard} 名，继续打卡就能进前 {PREVIEW}
        </p>
      )}

      {board.entries.length > PREVIEW && (
        <button
          onClick={() => setExpanded(!expanded)}
          className="mt-2 w-full text-xs text-white/50 underline underline-offset-4 active:text-white/80"
        >
          {expanded ? '收起' : `查看完整榜单（${board.entries.length} 人）`}
        </button>
      )}
    </section>
  )
}
