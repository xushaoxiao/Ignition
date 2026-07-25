import type { Dictionary } from "@/i18n/dictionaries";
import { ArrowRightIcon } from "../icons";
import { LinkButton } from "../link-button";

const POLICY_URL =
  "https://github.com/xushaoxiao/Ignition/blob/main/docs/product/attribution-policy-v1.md";

export function Hero({ dict }: { dict: Dictionary["hero"] }) {
  return (
    <section className="relative overflow-hidden">
      {/* backdrop */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 -z-10 text-black/[0.55] bg-grid dark:text-white/[0.6]"
      />
      <div
        aria-hidden
        className="pointer-events-none absolute inset-x-0 -top-40 -z-10 mx-auto h-[420px] max-w-4xl rounded-full bg-brand/20 blur-3xl"
      />
      <div className="mx-auto w-full max-w-6xl px-6 pb-16 pt-20 sm:pb-24 sm:pt-28">
        <div className="mx-auto max-w-3xl text-center">
          <span className="inline-flex items-center gap-2 rounded-full border border-brand/25 bg-brand/10 px-3 py-1 text-sm font-medium text-brand">
            <span className="size-1.5 rounded-full bg-brand" />
            {dict.badge}
          </span>

          <h1 className="mt-6 text-balance text-4xl font-semibold tracking-tight sm:text-6xl">
            {dict.titleLead} <span className="text-brand">{dict.titleEmph}</span>
          </h1>

          <p className="mx-auto mt-6 max-w-2xl text-pretty text-lg leading-relaxed text-black/65 dark:text-white/65">
            {dict.subtitle}
          </p>

          <div className="mt-9 flex flex-col items-center justify-center gap-3 sm:flex-row">
            <LinkButton href="#cta" variant="primary" size="lg">
              <span className="inline-flex items-center gap-2">
                {dict.ctaPrimary}
                <ArrowRightIcon className="size-4" />
              </span>
            </LinkButton>
            <LinkButton href={POLICY_URL} variant="outline" size="lg">
              {dict.ctaSecondary}
            </LinkButton>
          </div>

          <p className="mt-5 text-sm text-black/45 dark:text-white/45">{dict.note}</p>
        </div>

        <dl className="mx-auto mt-16 grid max-w-3xl grid-cols-1 gap-px overflow-hidden rounded-2xl border border-black/10 bg-black/10 sm:grid-cols-3 dark:border-white/10 dark:bg-white/10">
          {dict.stats.map((stat) => (
            <div
              key={stat.label}
              className="bg-background px-6 py-7 text-center"
            >
              <dt className="text-2xl font-semibold tracking-tight text-brand sm:text-3xl">
                {stat.value}
              </dt>
              <dd className="mt-1 text-sm text-black/55 dark:text-white/55">{stat.label}</dd>
            </div>
          ))}
        </dl>
      </div>
    </section>
  );
}
