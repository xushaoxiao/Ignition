import type { Dictionary } from "@/i18n/dictionaries";
import { Section, SectionHeading } from "../section";

export function HowItWorks({ dict }: { dict: Dictionary["how"] }) {
  return (
    <Section
      id="how"
      className="border-y border-black/5 bg-black/[0.015] dark:border-white/10 dark:bg-white/[0.02]"
    >
      <SectionHeading kicker={dict.kicker} title={dict.title} subtitle={dict.subtitle} />
      <ol className="mt-14 grid gap-4 md:grid-cols-5">
        {dict.steps.map((step, i) => (
          <li key={step.step} className="relative">
            <div className="flex h-full flex-col rounded-2xl border border-black/10 bg-background p-5 dark:border-white/10">
              <span className="font-mono text-sm font-semibold text-brand">{step.step}</span>
              <h3 className="mt-3 text-base font-semibold tracking-tight">{step.title}</h3>
              <p className="mt-2 text-pretty text-sm leading-relaxed text-black/60 dark:text-white/60">
                {step.desc}
              </p>
            </div>
            {i < dict.steps.length - 1 ? (
              <span
                aria-hidden
                className="absolute -right-2.5 top-1/2 z-10 hidden size-5 -translate-y-1/2 place-items-center rounded-full border border-black/10 bg-background text-black/40 md:grid dark:border-white/10 dark:text-white/40"
              >
                ›
              </span>
            ) : null}
          </li>
        ))}
      </ol>
    </Section>
  );
}
