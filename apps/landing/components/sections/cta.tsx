import type { Dictionary } from "@/i18n/dictionaries";
import { ArrowRightIcon } from "../icons";
import { LinkButton } from "../link-button";

const GITHUB_URL = "https://github.com/xushaoxiao/Ignition";
const DOCS_URL = "https://github.com/xushaoxiao/Ignition#readme";

export function Cta({ dict }: { dict: Dictionary["cta"] }) {
  return (
    <section id="cta" className="scroll-mt-20 px-6 py-20 sm:py-28">
      <div className="relative mx-auto w-full max-w-5xl overflow-hidden rounded-3xl border border-brand/30 bg-brand/[0.06] px-6 py-14 text-center sm:px-12">
        <div
          aria-hidden
          className="pointer-events-none absolute inset-x-0 -top-24 mx-auto h-64 max-w-2xl rounded-full bg-brand/25 blur-3xl"
        />
        <div className="relative">
          <h2 className="text-balance text-3xl font-semibold tracking-tight sm:text-4xl">
            {dict.title}
          </h2>
          <p className="mx-auto mt-4 max-w-2xl text-pretty text-base leading-relaxed text-black/65 dark:text-white/70">
            {dict.subtitle}
          </p>
          <div className="mt-9 flex flex-col items-center justify-center gap-3 sm:flex-row">
            <LinkButton href={GITHUB_URL} variant="primary" size="lg">
              <span className="inline-flex items-center gap-2">
                {dict.primary}
                <ArrowRightIcon className="size-4" />
              </span>
            </LinkButton>
            <LinkButton href={DOCS_URL} variant="outline" size="lg">
              {dict.secondary}
            </LinkButton>
          </div>
        </div>
      </div>
    </section>
  );
}
