import type { Dictionary } from "@/i18n/dictionaries";
import { CheckIcon } from "../icons";
import { LinkButton } from "../link-button";
import { Section, SectionHeading } from "../section";

export function Pricing({ dict }: { dict: Dictionary["pricing"] }) {
  return (
    <Section
      id="pricing"
      className="border-y border-black/5 bg-black/[0.015] dark:border-white/10 dark:bg-white/[0.02]"
    >
      <SectionHeading kicker={dict.kicker} title={dict.title} subtitle={dict.subtitle} />
      <div className="mt-14 grid items-start gap-6 lg:grid-cols-3">
        {dict.plans.map((plan) => {
          const highlighted = plan.highlighted;
          return (
            <div
              key={plan.name}
              className={
                "relative flex h-full flex-col rounded-3xl border p-7 " +
                (highlighted
                  ? "border-brand/50 bg-background shadow-[0_0_0_1px] shadow-brand/30 ring-1 ring-brand/20"
                  : "border-black/10 bg-background dark:border-white/10")
              }
            >
              {highlighted ? (
                <span className="absolute -top-3 left-7 rounded-full bg-brand px-3 py-1 text-xs font-semibold text-white">
                  {dict.mostPopular}
                </span>
              ) : null}
              <h3 className="text-lg font-semibold tracking-tight">{plan.name}</h3>
              <p className="mt-3 text-2xl font-semibold tracking-tight text-brand">{plan.price}</p>
              <p className="mt-2 text-sm text-black/55 dark:text-white/55">{plan.tagline}</p>

              <ul className="mt-6 flex-1 space-y-3">
                {plan.features.map((feature) => (
                  <li key={feature} className="flex items-start gap-2.5 text-sm">
                    <CheckIcon className="mt-0.5 size-4 shrink-0 text-brand" />
                    <span className="text-black/75 dark:text-white/75">{feature}</span>
                  </li>
                ))}
              </ul>

              <div className="mt-8">
                <LinkButton
                  href="#cta"
                  variant={highlighted ? "primary" : "outline"}
                  size="md"
                  fullWidth
                >
                  {plan.cta}
                </LinkButton>
              </div>
            </div>
          );
        })}
      </div>
    </Section>
  );
}
