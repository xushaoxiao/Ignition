/**
 * Mini app entry.
 *
 * ```text
 * open → session → (campaign's game)
 * ```
 *
 * The session tells us which game the campaign runs (`template.code`). Two shapes exist:
 *
 * - **prize draw** — wheel, scratch card, slot machine, blind box, flip cards. Animation skins
 *   over one server-decided outcome; see {@link PrizeFlow}.
 * - **daily decision** — `daily_budget`. A scored decision per day with a streak and a
 *   leaderboard, which then hands off to the same prize draw; see {@link DailyApp}.
 *
 * Both end at the claim code, which is the only billable step. Everything before it is
 * acquisition and retention theatre — deliberately so.
 */
import { useEffect, useState } from 'react'
import { openSession, type Session } from './api'
import { DailyApp } from './daily/DailyApp'
import { PrizeFlow } from './PrizeFlow'
import { inTelegram, rawInitData, setupTelegram } from './telegram'
import { Centered, Shell, messageOf } from './ui'

// Cosmetic override to preview a specific game (?game=slot_machine). The outcome always comes from
// the server; this only swaps the presentation, so it is harmless outside previews.
const GAME_OVERRIDE = new URLSearchParams(window.location.search).get('game')

/** Template code of the daily budget-decision game. */
const DAILY_BUDGET = 'daily_budget'

type Phase = 'booting' | 'ready' | 'fatal'

export default function App() {
  const [phase, setPhase] = useState<Phase>('booting')
  const [session, setSession] = useState<Session | null>(null)
  const [error, setError] = useState<string | null>(null)

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
        setSession(await openSession(initData))
        setPhase('ready')
      } catch (e) {
        setError(messageOf(e))
        setPhase('fatal')
      }
    })()
  }, [])

  if (phase === 'booting') {
    return (
      <Shell>
        <Centered>正在进入…</Centered>
      </Shell>
    )
  }

  if (phase === 'fatal' || !session) {
    return (
      <Shell>
        <Centered>
          <p className="text-white/80">{error}</p>
        </Centered>
      </Shell>
    )
  }

  const game = GAME_OVERRIDE ?? session.game
  return (
    <Shell>
      {game === DAILY_BUDGET ? (
        <DailyApp session={session} />
      ) : (
        <PrizeFlow session={session} gameCode={game} />
      )}
    </Shell>
  )
}
