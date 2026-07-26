import type { ReactNode } from "react";

export const inputClass =
  "w-full rounded-xl border border-black/15 bg-white px-3.5 py-2.5 text-sm text-black outline-none transition-colors placeholder:text-black/35 focus:border-brand focus:ring-2 focus:ring-brand/20 dark:border-white/15 dark:bg-white/5 dark:text-white dark:placeholder:text-white/30";

export function Panel({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={`rounded-2xl border border-black/10 bg-white p-6 dark:border-white/10 dark:bg-white/[0.03] ${className}`}
    >
      {children}
    </div>
  );
}

export function SectionTitle({ children, step }: { children: ReactNode; step?: number }) {
  return (
    <div className="mb-4 flex items-center gap-2.5">
      {step !== undefined ? (
        <span className="grid size-6 place-items-center rounded-full bg-brand text-xs font-bold text-white">
          {step}
        </span>
      ) : null}
      <h2 className="text-sm font-semibold uppercase tracking-wide text-black/60 dark:text-white/60">
        {children}
      </h2>
    </div>
  );
}

export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-sm font-medium">{label}</span>
      {children}
      {hint ? <span className="mt-1 block text-xs text-black/45 dark:text-white/45">{hint}</span> : null}
    </label>
  );
}
