"use client";

import { Button } from "@heroui/react";
import type { Prize } from "@/lib/types";
import { inputClass } from "./ui";

let seq = 0;
function newPrizeId(): string {
  seq += 1;
  return `p_${seq}_${Math.floor(Math.random() * 1e6)}`;
}

export function makePrize(label = "", weight = 10, remaining = 100): Prize {
  return { id: newPrizeId(), label, weight, remaining };
}

export function PrizeEditor({
  prizes,
  onChange,
}: {
  prizes: Prize[];
  onChange: (prizes: Prize[]) => void;
}) {
  const totalWeight = prizes.reduce((s, p) => s + Math.max(0, p.weight), 0);

  function update(id: string, patch: Partial<Prize>) {
    onChange(prizes.map((p) => (p.id === id ? { ...p, ...patch } : p)));
  }
  function remove(id: string) {
    onChange(prizes.filter((p) => p.id !== id));
  }
  function add() {
    onChange([...prizes, makePrize()]);
  }

  return (
    <div>
      <div className="mb-2 grid grid-cols-[1fr_5rem_5rem_3.5rem_2rem] items-center gap-2 px-1 text-xs font-medium text-black/45 dark:text-white/45">
        <span>奖品名称</span>
        <span className="text-center">权重</span>
        <span className="text-center">库存</span>
        <span className="text-center">中奖率</span>
        <span />
      </div>

      <div className="flex flex-col gap-2">
        {prizes.map((p) => {
          const pct = totalWeight > 0 ? Math.round((Math.max(0, p.weight) / totalWeight) * 1000) / 10 : 0;
          return (
            <div
              key={p.id}
              className="grid grid-cols-[1fr_5rem_5rem_3.5rem_2rem] items-center gap-2"
            >
              <input
                className={inputClass}
                value={p.label}
                placeholder="如：100 金币"
                onChange={(e) => update(p.id, { label: e.target.value })}
              />
              <input
                type="number"
                min={0}
                className={`${inputClass} text-center`}
                value={p.weight}
                onChange={(e) => update(p.id, { weight: Math.max(0, Number(e.target.value) || 0) })}
              />
              <input
                type="number"
                min={0}
                className={`${inputClass} text-center`}
                value={p.remaining}
                onChange={(e) => update(p.id, { remaining: Math.max(0, Number(e.target.value) || 0) })}
              />
              <span className="text-center text-sm tabular-nums text-black/60 dark:text-white/60">
                {pct}%
              </span>
              <button
                type="button"
                aria-label="删除奖品"
                onClick={() => remove(p.id)}
                disabled={prizes.length <= 1}
                className="grid size-8 place-items-center rounded-lg text-black/40 transition-colors hover:bg-red-500/10 hover:text-red-500 disabled:opacity-30 disabled:hover:bg-transparent dark:text-white/40"
              >
                ✕
              </button>
            </div>
          );
        })}
      </div>

      <div className="mt-3 flex items-center justify-between">
        <Button variant="outline" size="sm" onPress={add}>
          + 添加奖品
        </Button>
        <span className="text-xs text-black/45 dark:text-white/45">
          总权重 {totalWeight} · 中奖率按权重实时计算
        </span>
      </div>
    </div>
  );
}
