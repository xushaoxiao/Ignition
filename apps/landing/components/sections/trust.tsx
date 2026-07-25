import type { Dictionary } from "@/i18n/dictionaries";
import { Section, SectionHeading } from "../section";

export function Trust({ dict }: { dict: Dictionary["trust"] }) {
  return (
    <Section id="trust">
      <SectionHeading kicker={dict.kicker} title={dict.title} subtitle={dict.subtitle} />
      <div className="mt-14 grid gap-5 md:grid-cols-2">
        {dict.items.map((item) => (
          <div
            key={item.code}
            className="relative overflow-hidden rounded-2xl border border-black/10 bg-background p-7 dark:border-white/10"
          >
            <span
              aria-hidden
              className="pointer-events-none absolute -right-3 -top-4 select-none font-mono text-7xl font-bold text-brand/10"
            >
              {item.code}
            </span>
            <div className="relative">
              <span className="inline-flex items-center rounded-full bg-brand/12 px-2.5 py-1 font-mono text-xs font-semibold text-brand">
                {item.code}
              </span>
              <h3 className="mt-4 text-xl font-semibold tracking-tight">{item.title}</h3>
              <p className="mt-2 text-pretty text-sm leading-relaxed text-black/60 dark:text-white/60">
                {item.desc}
              </p>
            </div>
          </div>
        ))}
      </div>
    </Section>
  );
}
