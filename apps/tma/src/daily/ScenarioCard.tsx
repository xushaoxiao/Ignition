/**
 * Today's scenario and its options.
 *
 * The options carry no hint of their score — the server does not send one, and the card must not
 * invent one (no colour coding, no ordering by "goodness"). The whole point of the format is that
 * the player commits before seeing the answer.
 */
import type { DailyScenario } from '../api'

const LETTERS = ['A', 'B', 'C', 'D', 'E', 'F']

interface Props {
  scenario: DailyScenario
  /** Key currently being submitted; the others lock so a double tap cannot answer twice. */
  pending: string | null
  onPick: (key: string) => void
}

export function ScenarioCard({ scenario, pending, onPick }: Props) {
  const locked = pending !== null

  return (
    <section className="flex flex-col gap-4">
      <div className="rounded-2xl bg-white/8 p-4 ring-1 ring-white/12">
        <h2 className="text-lg font-bold">{scenario.title}</h2>
        <p className="mt-2 text-[15px] leading-relaxed text-white/75">{scenario.prompt}</p>
      </div>

      <div className="flex flex-col gap-2.5">
        {scenario.choices.map((c, i) => (
          <button
            key={c.key}
            onClick={() => onPick(c.key)}
            disabled={locked}
            className={`flex w-full items-center gap-3 rounded-2xl px-4 py-3.5 text-left ring-1 transition-colors ${
              pending === c.key
                ? 'bg-amber-400/90 text-slate-900 ring-amber-300'
                : 'bg-white/8 text-white ring-white/12 active:bg-white/16 disabled:opacity-45'
            }`}
          >
            <span
              className={`grid size-7 shrink-0 place-items-center rounded-full text-xs font-bold ${
                pending === c.key ? 'bg-slate-900/15 text-slate-900' : 'bg-white/12 text-white/70'
              }`}
            >
              {LETTERS[i] ?? i + 1}
            </span>
            <span className="text-[15px] leading-snug">{c.label}</span>
          </button>
        ))}
      </div>
    </section>
  )
}
