"use client";

import { Accordion } from "@heroui/react";
import type { Dictionary } from "@/i18n/dictionaries";

function ChevronDown({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
    >
      <path d="M6 9l6 6 6-6" />
    </svg>
  );
}

export function FaqAccordion({ items }: { items: Dictionary["faq"]["items"] }) {
  return (
    <Accordion className="mx-auto w-full max-w-3xl">
      {items.map((item, index) => (
        <Accordion.Item key={index}>
          <Accordion.Heading>
            <Accordion.Trigger className="text-left text-base font-medium">
              {item.q}
              <Accordion.Indicator>
                <ChevronDown className="size-4" />
              </Accordion.Indicator>
            </Accordion.Trigger>
          </Accordion.Heading>
          <Accordion.Panel>
            <Accordion.Body className="text-pretty text-sm leading-relaxed text-black/60 dark:text-white/60">
              {item.a}
            </Accordion.Body>
          </Accordion.Panel>
        </Accordion.Item>
      ))}
    </Accordion>
  );
}
