/**
 * Daily budget-decision game.
 *
 * ```text
 * 今日场景 → 选择 → 评分 + 科普 → 排行榜 → 抽奖 → 兑换码
 * ```
 *
 * The decision round is the retention half (a reason to come back tomorrow); the prize draw it
 * leads into is the acquisition half, and the claim code at the end is the only billable step.
 *
 * **The handoff to the draw is a UI sequence, not a server-side entitlement.** Answering does not
 * grant a play, and `daily_play_limit` still decides how many draws a player gets. That is
 * deliberate: if a score the player can influence could unlock extra draws, the engagement layer
 * would start moving prize cost and, downstream, invoices — exactly what constraint C1 forbids.
 */
import { useEffect, useState } from 'react'
import {
  dailyAnswer,
  dailyLeaderboard,
  dailyToday,
  type DailyLeaderboard,
  type DailyOutcome,
  type DailyToday,
  type Promo,
  type Session,
} from '../api'
import { PrizeFlow } from '../PrizeFlow'
import { shareInvite, successFeedback, tapFeedback } from '../telegram'
import { Centered, messageOf } from '../ui'
import { CreditMeter } from './CreditMeter'
import { Leaderboard } from './Leaderboard'
import { OutcomeCard } from './OutcomeCard'
import { ScenarioCard } from './ScenarioCard'

/**
 * Animation skin for the reward draw.
 *
 * The campaign's own template code (`daily_budget`) names a decision game, not an animation, so
 * the draw needs one chosen here. A box that opens fits the "今日奖励" beat better than a wheel,
 * which reads as a separate lottery.
 */
const REWARD_SKIN = 'blind_box'

type Stage = 'loading' | 'decide' | 'result' | 'prize' | 'fatal'

export function DailyApp({ session }: { session: Session }) {
  const [stage, setStage] = useState<Stage>('loading')
  const [today, setToday] = useState<DailyToday | null>(null)
  const [outcome, setOutcome] = useState<DailyOutcome | null>(null)
  const [promo, setPromo] = useState<Promo | null>(null)
  const [rank, setRank] = useState<number | null>(null)
  const [players, setPlayers] = useState(0)
  const [board, setBoard] = useState<DailyLeaderboard | null>(null)
  const [pending, setPending] = useState<string | null>(null)
  const [replayed, setReplayed] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    void (async () => {
      try {
        const t = await dailyToday()
        setToday(t)
        setPromo(t.promo)
        setRank(t.rank)
        setPlayers(t.players)
        // Reopening after answering lands straight on the result. Offering the question again
        // would only end in a rejection the player cannot act on — the day is already recorded.
        if (t.answered) {
          setOutcome(t.answered)
          setReplayed(true)
          setStage('result')
          void loadBoard()
        } else {
          setStage('decide')
        }
      } catch (e) {
        setError(messageOf(e))
        setStage('fatal')
      }
    })()
  }, [])

  async function loadBoard() {
    try {
      setBoard(await dailyLeaderboard())
    } catch {
      // The board is decoration; failing to load it must not block the prize draw behind it.
    }
  }

  async function pick(key: string) {
    if (pending) return
    setPending(key)
    setError(null)
    tapFeedback()
    try {
      const r = await dailyAnswer(key)
      successFeedback()
      setOutcome(r)
      setPromo(r.promo)
      setRank(r.rank)
      setPlayers(r.players)
      setReplayed(r.idempotent)
      setStage('result')
      void loadBoard()
    } catch (e) {
      // Stay on the question: an answer that failed on the network was not recorded, and the
      // player can pick again. A dead end here costs the whole session.
      setError(messageOf(e))
    } finally {
      setPending(null)
    }
  }

  if (stage === 'loading') return <Centered>正在加载今日场景…</Centered>

  if (stage === 'fatal' || !today) {
    return (
      <Centered>
        <p className="text-white/80">{error}</p>
      </Centered>
    )
  }

  if (stage === 'prize') {
    return <PrizeFlow session={session} gameCode={REWARD_SKIN} note="今日决策已完成" />
  }

  const credit = outcome?.credit ?? today.credit
  const grade = outcome?.grade ?? today.grade
  const gradeLabel = outcome?.grade_label ?? today.grade_label
  const streak = outcome?.streak ?? today.streak

  return (
    <>
      <header className="text-center">
        <h1 className="text-xl font-bold">每日理财决策</h1>
        <p className="mt-1 text-sm text-white/60">
          {/* `rounds_played` counts completed rounds, so today's own round is the next one until
              the server confirms it is recorded. */}
          {formatDay(today.date)} · 第 {today.rounds_played + (today.answered ? 0 : 1)} 期
        </p>
      </header>

      <CreditMeter
        credit={credit}
        grade={grade}
        gradeLabel={gradeLabel}
        streak={streak}
        rank={rank}
        players={players}
      />

      {stage === 'decide' && (
        <>
          <ScenarioCard scenario={today.scenario} pending={pending} onPick={pick} />
          {error && (
            <p className="rounded-xl bg-red-500/15 px-4 py-3 text-center text-sm text-red-200">
              {error}
            </p>
          )}
        </>
      )}

      {stage === 'result' && outcome && (
        <>
          <OutcomeCard outcome={outcome} promo={promo} replayed={replayed} />
          <Leaderboard board={board} />

          <div className="flex flex-col gap-2.5">
            {session.plays_left > 0 ? (
              <button
                onClick={() => setStage('prize')}
                className="w-full rounded-2xl bg-amber-400 py-4 text-lg font-bold text-slate-900 active:bg-amber-500"
              >
                🎁 领取今日奖励抽奖
              </button>
            ) : (
              <p className="rounded-2xl bg-white/8 py-3.5 text-center text-sm text-white/60 ring-1 ring-white/12">
                今日抽奖次数已用完，明天再来
              </p>
            )}
            <button
              onClick={() =>
                shareInvite(
                  `📊 我今天的理财分 ${credit}（${gradeLabel}），已连续打卡 ${streak} 天。来试试今天的理财决策：`,
                )
              }
              className="w-full rounded-2xl bg-white/10 py-3 text-sm font-semibold text-white/90 ring-1 ring-white/15 active:bg-white/20"
            >
              🤝 晒成绩，喊好友来挑战
            </button>
            <p className="text-center text-xs text-white/40">明天 0 点更新新场景，连续打卡额外加分</p>
          </div>
        </>
      )}
    </>
  )
}

/** `2026-08-04` → `8 月 4 日`. The server sends a plain date, not a timestamp, so no parsing risk. */
function formatDay(iso: string): string {
  const [, m, d] = iso.split('-')
  if (!m || !d) return iso
  return `${Number(m)} 月 ${Number(d)} 日`
}
