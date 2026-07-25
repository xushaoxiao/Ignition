"use client";

import { usePathname, useRouter } from "next/navigation";
import { locales, type Locale } from "@/i18n/config";

const short: Record<Locale, string> = { zh: "中", en: "EN" };

export function LanguageSwitch({ locale }: { locale: Locale }) {
  const pathname = usePathname();
  const router = useRouter();

  function switchTo(next: Locale) {
    if (next === locale) return;
    const segments = pathname.split("/");
    segments[1] = next; // first segment after the leading slash is the locale
    router.push(segments.join("/") || "/");
  }

  return (
    <div
      role="group"
      aria-label="Language"
      className="inline-flex items-center rounded-full border border-black/10 bg-black/[0.03] p-0.5 text-xs font-medium dark:border-white/15 dark:bg-white/[0.04]"
    >
      {locales.map((l) => {
        const active = l === locale;
        return (
          <button
            key={l}
            type="button"
            aria-pressed={active}
            onClick={() => switchTo(l)}
            className={
              "rounded-full px-2.5 py-1 transition-colors " +
              (active
                ? "bg-brand text-white shadow-sm"
                : "text-black/55 hover:text-black dark:text-white/55 dark:hover:text-white")
            }
          >
            {short[l]}
          </button>
        );
      })}
    </div>
  );
}
