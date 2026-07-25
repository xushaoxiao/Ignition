"use client";

import { Button } from "@heroui/react";
import { useState } from "react";
import type { Dictionary } from "@/i18n/dictionaries";
import type { Locale } from "@/i18n/config";
import { CloseIcon, MenuIcon, SparkIcon } from "./icons";
import { LanguageSwitch } from "./language-switch";
import { LinkButton } from "./link-button";
import { ThemeSwitch } from "./theme-switch";

const DOCS_URL = "https://github.com/xushaoxiao/Ignition";

export function SiteHeader({ dict, locale }: { dict: Dictionary; locale: Locale }) {
  const [open, setOpen] = useState(false);
  const nav = dict.nav;

  const links = [
    { label: nav.features, href: "#features" },
    { label: nav.how, href: "#how" },
    { label: nav.trust, href: "#trust" },
    { label: nav.pricing, href: "#pricing" },
    { label: nav.faq, href: "#faq" },
  ];

  return (
    <header className="sticky top-0 z-50 border-b border-black/5 bg-background/70 backdrop-blur-xl dark:border-white/10">
      <div className="mx-auto flex h-16 w-full max-w-6xl items-center justify-between px-6">
        <a href={`/${locale}`} className="flex items-center gap-2 font-semibold tracking-tight">
          <span className="grid size-8 place-items-center rounded-lg bg-brand/12 text-brand">
            <SparkIcon className="size-5" />
          </span>
          <span className="text-lg">Ignition</span>
        </a>

        <nav className="hidden items-center gap-1 md:flex">
          {links.map((l) => (
            <a
              key={l.href}
              href={l.href}
              className="rounded-lg px-3 py-2 text-sm text-black/65 transition-colors hover:bg-black/5 hover:text-black dark:text-white/65 dark:hover:bg-white/5 dark:hover:text-white"
            >
              {l.label}
            </a>
          ))}
          <a
            href={DOCS_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="rounded-lg px-3 py-2 text-sm text-black/65 transition-colors hover:bg-black/5 hover:text-black dark:text-white/65 dark:hover:bg-white/5 dark:hover:text-white"
          >
            {nav.docs}
          </a>
        </nav>

        <div className="flex items-center gap-2">
          <div className="hidden items-center gap-2 sm:flex">
            <LanguageSwitch locale={locale} />
            <ThemeSwitch />
          </div>
          <div className="hidden md:block">
            <LinkButton href="#cta" variant="primary" size="sm">
              {nav.cta}
            </LinkButton>
          </div>
          <div className="md:hidden">
            <Button
              isIconOnly
              variant="ghost"
              size="sm"
              aria-label="Toggle menu"
              aria-expanded={open}
              onPress={() => setOpen((v) => !v)}
            >
              {open ? <CloseIcon className="size-5" /> : <MenuIcon className="size-5" />}
            </Button>
          </div>
        </div>
      </div>

      {open ? (
        <div className="border-t border-black/5 bg-background/95 px-6 py-4 md:hidden dark:border-white/10">
          <nav className="flex flex-col gap-1">
            {[...links, { label: nav.docs, href: DOCS_URL }].map((l) => (
              <a
                key={l.href}
                href={l.href}
                onClick={() => setOpen(false)}
                className="rounded-lg px-3 py-2.5 text-sm text-black/70 hover:bg-black/5 dark:text-white/70 dark:hover:bg-white/5"
              >
                {l.label}
              </a>
            ))}
          </nav>
          <div className="mt-4 flex items-center justify-between">
            <LanguageSwitch locale={locale} />
            <div className="flex items-center gap-2">
              <ThemeSwitch />
              <LinkButton href="#cta" variant="primary" size="sm">
                {nav.cta}
              </LinkButton>
            </div>
          </div>
        </div>
      ) : null}
    </header>
  );
}
