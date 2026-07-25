import type { ComponentType, SVGProps } from "react";
import type { Dictionary } from "@/i18n/dictionaries";
import {
  GameIcon,
  GaugeIcon,
  LedgerIcon,
  PolicyIcon,
  ShieldIcon,
  TargetIcon,
} from "../icons";
import { Section, SectionHeading } from "../section";

const icons: ComponentType<SVGProps<SVGSVGElement>>[] = [
  TargetIcon,
  LedgerIcon,
  PolicyIcon,
  GameIcon,
  ShieldIcon,
  GaugeIcon,
];

export function Features({ dict }: { dict: Dictionary["features"] }) {
  return (
    <Section id="features">
      <SectionHeading kicker={dict.kicker} title={dict.title} subtitle={dict.subtitle} />
      <div className="mt-14 grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
        {dict.items.map((item, i) => {
          const Icon = icons[i] ?? TargetIcon;
          return (
            <div
              key={item.title}
              className="group rounded-2xl border border-black/10 bg-background p-6 transition-colors hover:border-brand/40 dark:border-white/10"
            >
              <span className="grid size-11 place-items-center rounded-xl bg-brand/12 text-brand transition-transform group-hover:scale-105">
                <Icon className="size-5" />
              </span>
              <h3 className="mt-5 text-lg font-semibold tracking-tight">{item.title}</h3>
              <p className="mt-2 text-pretty text-sm leading-relaxed text-black/60 dark:text-white/60">
                {item.desc}
              </p>
            </div>
          );
        })}
      </div>
    </Section>
  );
}
