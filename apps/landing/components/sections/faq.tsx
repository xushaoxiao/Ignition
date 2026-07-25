import type { Dictionary } from "@/i18n/dictionaries";
import { Section, SectionHeading } from "../section";
import { FaqAccordion } from "./faq-accordion";

export function Faq({ dict }: { dict: Dictionary["faq"] }) {
  return (
    <Section id="faq">
      <SectionHeading kicker={dict.kicker} title={dict.title} />
      <div className="mt-12">
        <FaqAccordion items={dict.items} />
      </div>
    </Section>
  );
}
