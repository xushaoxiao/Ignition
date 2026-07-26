"use client";

import { GAMES } from "@/lib/games";

export function GamePicker({ value, onChange }: { value: string; onChange: (code: string) => void }) {
  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
      {GAMES.map((g) => {
        const active = g.code === value;
        return (
          <button
            key={g.code}
            type="button"
            onClick={() => onChange(g.code)}
            aria-pressed={active}
            className={
              "flex flex-col items-start rounded-2xl border p-4 text-left transition-colors " +
              (active
                ? "border-brand bg-brand/5 ring-1 ring-brand/30"
                : "border-black/10 hover:border-brand/40 dark:border-white/10")
            }
          >
            <span className="text-2xl">{g.emoji}</span>
            <span className="mt-2 text-sm font-semibold">{g.title}</span>
            <span className="mt-1 text-xs leading-snug text-black/50 dark:text-white/50">
              {g.blurb}
            </span>
          </button>
        );
      })}
    </div>
  );
}
