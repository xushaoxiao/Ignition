/**
 * Result of today's decision: verdict, score breakdown, and the teaching line.
 *
 * The tip is the reason this game exists rather than being one more tap-to-win — it is what makes
 * a wrong answer worth having given. So it is not a footnote: it gets its own block, and it is
 * shown for good answers too.
 *
 * Two things are load-bearing and should not be quietly dropped:
 *
 * - **The disclaimer.** Every line of copy here is generic financial literacy, not advice about
 *   anyone's situation. Saying so plainly costs one line and keeps the game honest.
 * - **The promo is labelled as the campaign's.** It only appears above the score threshold the
 *   customer configured, it renders under a divider, and it never borrows the voice of the tips
 *   above it.
 */
import type { DailyOutcome, Promo } from '../api'
import { signed } from './score'

interface Props {
  outcome: DailyOutcome
  promo: Promo | null
  /** True when the round was already answered before this visit (a same-day reopen). */
  replayed: boolean
}

export function OutcomeCard({ outcome, promo, replayed }: Props) {
  const good = outcome.delta >= 8

  return (
    <section className="flex flex-col gap-4">
      <div className="rounded-2xl bg-white/8 p-4 ring-1 ring-white/12">
        <p className="text-xs text-white/55">{replayed ? '你今天的选择' : '你的选择'}</p>
        <p className="mt-1 text-[15px] font-semibold">{outcome.choice_label}</p>

        <div className="mt-3 flex flex-wrap gap-2">
          <span
            className={`rounded-full px-3 py-1 text-sm font-bold ${
              outcome.delta >= 0
                ? 'bg-emerald-400/15 text-emerald-300'
                : 'bg-rose-400/15 text-rose-300'
            }`}
          >
            决策 {signed(outcome.delta)}
          </span>
          {outcome.streak_bonus > 0 && (
            <span className="rounded-full bg-amber-400/15 px-3 py-1 text-sm font-bold text-amber-300">
              连续 {outcome.streak} 天 {signed(outcome.streak_bonus)}
            </span>
          )}
        </div>

        <p className={`mt-3 text-[15px] leading-relaxed ${good ? 'text-emerald-200' : 'text-white/80'}`}>
          {outcome.verdict}
        </p>
      </div>

      <div className="rounded-2xl bg-sky-400/8 p-4 ring-1 ring-sky-300/20">
        <p className="text-xs font-semibold text-sky-300">💡 为什么</p>
        <p className="mt-1.5 text-[15px] leading-relaxed text-white/80">{outcome.tip}</p>
      </div>

      <p className="px-1 text-[11px] leading-relaxed text-white/35">
        以上内容为通用理财科普，不构成投资、借贷或税务建议；请结合自身情况判断。
      </p>

      {promo && (
        <a
          href={promo.url ?? '#'}
          target="_blank"
          rel="noreferrer"
          className="rounded-2xl border border-dashed border-white/20 px-4 py-3 text-center text-sm text-white/70 active:bg-white/10"
        >
          {promo.text} →
        </a>
      )}
    </section>
  )
}
