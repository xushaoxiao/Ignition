import type { Dictionary } from "@/i18n/dictionaries";
import { GithubIcon, SparkIcon } from "./icons";

const GITHUB_URL = "https://github.com/xushaoxiao/Ignition";

export function SiteFooter({ dict }: { dict: Dictionary["footer"] }) {
  return (
    <footer className="border-t border-black/10 dark:border-white/10">
      <div className="mx-auto w-full max-w-6xl px-6 py-14">
        <div className="grid gap-10 md:grid-cols-[1.5fr_repeat(3,1fr)]">
          <div>
            <div className="flex items-center gap-2 font-semibold tracking-tight">
              <span className="grid size-8 place-items-center rounded-lg bg-brand/12 text-brand">
                <SparkIcon className="size-5" />
              </span>
              <span className="text-lg">Ignition</span>
            </div>
            <p className="mt-4 max-w-xs text-pretty text-sm leading-relaxed text-black/55 dark:text-white/55">
              {dict.tagline}
            </p>
          </div>

          {dict.columns.map((column) => (
            <div key={column.title}>
              <h3 className="text-sm font-semibold">{column.title}</h3>
              <ul className="mt-4 space-y-2.5">
                {column.links.map((link) => {
                  const external = /^https?:\/\//.test(link.href);
                  return (
                    <li key={link.label}>
                      <a
                        href={link.href}
                        target={external ? "_blank" : undefined}
                        rel={external ? "noopener noreferrer" : undefined}
                        className="text-sm text-black/55 transition-colors hover:text-brand dark:text-white/55"
                      >
                        {link.label}
                      </a>
                    </li>
                  );
                })}
              </ul>
            </div>
          ))}
        </div>

        <div className="mt-12 flex flex-col items-center justify-between gap-4 border-t border-black/10 pt-6 sm:flex-row dark:border-white/10">
          <p className="text-sm text-black/45 dark:text-white/45">{dict.rights}</p>
          <a
            href={GITHUB_URL}
            target="_blank"
            rel="noopener noreferrer"
            aria-label="GitHub"
            className="grid size-9 place-items-center rounded-lg border border-black/10 text-black/60 transition-colors hover:border-brand/40 hover:text-brand dark:border-white/10 dark:text-white/60"
          >
            <GithubIcon className="size-4.5" />
          </a>
        </div>
      </div>
    </footer>
  );
}
