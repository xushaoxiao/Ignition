import type { Dictionary } from "@/i18n/dictionaries";
import { CheckIcon } from "../icons";
import { Section } from "../section";

export function Positioning({ dict }: { dict: Dictionary["positioning"] }) {
  return (
    <Section className="border-y border-black/5 bg-black/[0.015] dark:border-white/10 dark:bg-white/[0.02]">
      <div className="grid items-center gap-12 lg:grid-cols-2">
        <div>
          <p className="text-sm font-semibold uppercase tracking-widest text-brand">
            {dict.kicker}
          </p>
          <h2 className="mt-3 text-balance text-3xl font-semibold tracking-tight sm:text-4xl">
            {dict.title}
          </h2>
          <p className="mt-5 text-pretty text-base leading-relaxed text-black/60 dark:text-white/60">
            {dict.body}
          </p>
        </div>
        <ul className="space-y-4">
          {dict.points.map((point) => (
            <li
              key={point}
              className="flex items-start gap-3 rounded-2xl border border-black/10 bg-background p-5 dark:border-white/10"
            >
              <span className="mt-0.5 grid size-6 shrink-0 place-items-center rounded-full bg-brand/12 text-brand">
                <CheckIcon className="size-4" />
              </span>
              <span className="text-pretty text-sm leading-relaxed text-black/75 dark:text-white/75">
                {point}
              </span>
            </li>
          ))}
        </ul>
      </div>
    </Section>
  );
}
