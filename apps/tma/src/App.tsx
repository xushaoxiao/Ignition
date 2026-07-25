/**
 * Main wheel mini app flow.
 *
 * ```text
 * open → session → wheel → play → result → claim
 * ```
 *
 * Only one step on this path produces a billable conversion: claiming the code. Every
 * segment is designed not to strand the user — failures offer retry instead of dead
 * ends; after a win, claim runs automatically instead of asking for another tap.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import {
  ApiError,
  claim as claimApi,
  newIdempotencyKey,
  openSession,
  play as playApi,
  type ClaimResult,
  type PlayResult,
  type Session,
} from './api'
import { ClaimCard } from './components/ClaimCard'
import { Wheel } from './components/Wheel'
import { inTelegram, rawInitData, setupTelegram, shareInvite, successFeedback, tapFeedback } from './telegram'

type Phase = 'booting' | 'ready' | 'spinning' | 'claiming' | 'done' | 'fatal'

export default function App() {
  const [phase, setPhase] = useState<Phase>('booting')
  const [session, setSession] = useState<Session | null>(null)
  const [playsLeft, setPlaysLeft] = useState(0)
  const [result, setResult] = useState<PlayResult | null>(null)
  const [claim, setClaim] = useState<ClaimResult | null>(null)
  const [error, setError] = useState<string | null>(null)

  // Retries for the same tap must reuse the same idempotency key; the next tap gets a new one.
  const idemKey = useRef<string>('')

  useEffect(() => {
    void (async () => {
      await setupTelegram()
      const initData = await rawInitData()
      if (!initData) {
        setError(
          inTelegram()
            ? '登录信息缺失，请重新打开小程序'
            : '请在 Telegram 里通过群内链接打开本页面',
        )
        setPhase('fatal')
        return
      }
      try {
        const s = await openSession(initData)
        setSession(s)
        setPlaysLeft(s.plays_left)
        setPhase('ready')
      } catch (e) {
        setError(messageOf(e))
        setPhase('fatal')
      }
    })()
  }, [])

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

  // Claim immediately after the wheel stops. No extra "claim" tap —
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

  /** Back to the wheel for another spin. The previous code was already issued; users can find it in history. */
  function spinAgain() {
    setResult(null)
    setClaim(null)
    setError(null)
    setPhase('ready')
  }

  const segments = session?.prizes ?? []

  return (
    <main className="mx-auto flex min-h-dvh w-full max-w-md flex-col justify-center gap-7 px-5 pb-[calc(1.5rem+env(safe-area-inset-bottom))] pt-[calc(1.5rem+env(safe-area-inset-top))]">
      <header className="text-center">
        <h1 className="text-xl font-bold">幸运转盘</h1>
        {phase !== 'fatal' && phase !== 'booting' && (
          <p className="mt-1 text-sm text-white/60">今日还可抽 {playsLeft} 次</p>
        )}
      </header>

      {phase === 'booting' && <Centered>正在进入…</Centered>}

      {phase === 'fatal' && (
        <Centered>
          <p className="text-white/80">{error}</p>
        </Centered>
      )}

      {(phase === 'ready' || phase === 'spinning' || phase === 'claiming') && (
        <>
          <Wheel
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
                {playsLeft <= 0 ? '今日次数已用完' : phase === 'spinning' ? '转动中…' : '开始抽奖'}
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
    </main>
  )
}

function Centered({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex flex-1 items-center justify-center text-center text-white/70">
      {children}
    </div>
  )
}

/** Backend error messages are already end-user facing; pass them through. */
function messageOf(e: unknown): string {
  if (e instanceof ApiError) return e.message
  return '网络异常，请稍后再试'
}
