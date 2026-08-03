/**
 * Prize draw flow.
 *
 * ```text
 * play → reveal → claim
 * ```
 *
 * Only one step on this path produces a billable conversion: claiming the code. Every
 * segment is designed not to strand the user — failures offer retry instead of dead
 * ends; after a win, claim runs automatically instead of asking for another tap.
 *
 * Reached two ways: directly, for wheel-style campaigns, and as the reward stage of the daily
 * decision game. Both go through the same server-authoritative `play`, so which one led here
 * changes nothing about the draw, the claim code, or attribution.
 */
import { useCallback, useRef, useState } from 'react'
import {
  claim as claimApi,
  newIdempotencyKey,
  play as playApi,
  type ClaimResult,
  type PlayResult,
  type Session,
} from './api'
import { ClaimCard } from './components/ClaimCard'
import { gameFor } from '@ignition/games'
import { shareInvite, successFeedback, tapFeedback } from './telegram'
import { messageOf } from './ui'

type Phase = 'ready' | 'spinning' | 'claiming' | 'done'

interface Props {
  session: Session
  /**
   * Animation skin to use. Defaults to the campaign's own template code; the daily game passes
   * one explicitly because its template code names a decision game, not an animation.
   */
  gameCode?: string
  /**
   * Prefix for the line under the title, e.g. the daily game's "今日决策已完成". The remaining-plays
   * count is always appended from live state — a caller-supplied count would go stale after the
   * first draw and contradict the button below it.
   */
  note?: string
}

export function PrizeFlow({ session, gameCode, note }: Props) {
  const [phase, setPhase] = useState<Phase>('ready')
  const [playsLeft, setPlaysLeft] = useState(session.plays_left)
  const [result, setResult] = useState<PlayResult | null>(null)
  const [claim, setClaim] = useState<ClaimResult | null>(null)
  const [error, setError] = useState<string | null>(null)

  // Retries for the same tap must reuse the same idempotency key; the next tap gets a new one.
  const idemKey = useRef<string>('')

  async function spin() {
    if (phase !== 'ready' || playsLeft <= 0) return
    setError(null)
    idemKey.current = newIdempotencyKey()
    tapFeedback()
    setPhase('spinning')
    try {
      const r = await playApi(idemKey.current)
      setResult(r)
      setPlaysLeft(r.plays_left)
    } catch (e) {
      setError(messageOf(e))
      // Return to retryable state, not fatal: play failures are usually network or rate limits;
      // trapping the user on an error page wastes this conversion.
      setPhase('ready')
    }
  }

  // Claim immediately after the reveal settles. No extra "claim" tap —
  // every extra click loses users here, and this is where revenue lives.
  const onSettled = useCallback(() => {
    if (!result) return
    successFeedback()
    setPhase('claiming')
    void (async () => {
      try {
        setClaim(await claimApi(result.play_id))
        setPhase('done')
      } catch (e) {
        setError(messageOf(e))
        setPhase('claiming')
      }
    })()
  }, [result])

  /** Back to the game for another draw. The previous code was already issued; users can find it in history. */
  function spinAgain() {
    setResult(null)
    setClaim(null)
    setError(null)
    setPhase('ready')
  }

  const segments = session.prizes
  const game = gameFor(gameCode ?? session.game)
  const Game = game.Component

  return (
    <>
      <header className="text-center">
        <h1 className="text-xl font-bold">{game.title}</h1>
        <p className="mt-1 text-sm text-white/60">
          {note ? `${note} · ` : ''}今日还可抽 {playsLeft} 次
        </p>
      </header>

      {phase !== 'done' && (
        <>
          <Game
            segments={segments}
            target={result?.segment_index ?? null}
            spinning={phase === 'spinning' && result !== null}
            onSettled={onSettled}
          />

          {error && (
            <p className="rounded-xl bg-red-500/15 px-4 py-3 text-center text-sm text-red-200">
              {error}
            </p>
          )}

          {phase === 'claiming' && !error && (
            <p className="text-center text-sm text-white/60">正在生成兑换码…</p>
          )}

          {phase !== 'claiming' && (
            <div className="flex flex-col gap-2.5">
              <button
                onClick={spin}
                disabled={phase === 'spinning' || playsLeft <= 0}
                className="w-full rounded-2xl bg-amber-400 py-4 text-lg font-bold text-slate-900 disabled:bg-white/15 disabled:text-white/40 active:bg-amber-500"
              >
                {playsLeft <= 0 ? '今日次数已用完' : phase === 'spinning' ? '抽奖中…' : '开始抽奖'}
              </button>
              <button
                onClick={() => shareInvite('🎯 发现了一个超级好玩的 Telegram 幸运抽奖！邀请组团抽大奖：')}
                disabled={phase === 'spinning'}
                className="w-full rounded-2xl bg-white/10 py-3 text-sm font-semibold text-white/90 ring-1 ring-white/15 active:bg-white/20"
              >
                🤝 邀请好友组团 (+1 抽奖机会)
              </button>
            </div>
          )}
        </>
      )}

      {phase === 'done' && claim && result && (
        <>
          <ClaimCard claim={claim} prizeLabel={result.prize_label} />
          {playsLeft > 0 && (
            // Deliberately a de-emphasised secondary action: this screen's goal is to send
            // users into the main app to redeem — the only billable step. Hiding remaining
            // plays would make the campaign look one-and-done.
            <button
              onClick={spinAgain}
              className="text-sm text-white/50 underline underline-offset-4 active:text-white/80"
            >
              今日还可抽 {playsLeft} 次，再来一次
            </button>
          )}
        </>
      )}
    </>
  )
}
