"use client";

import { Button } from "@heroui/react";
import { gameFor, type Segment } from "@ignition/games";
import { useCallback, useState } from "react";
import type { Prize } from "@/lib/types";

/** Weighted pick for a realistic-feeling preview. Cosmetic only — the real outcome is server-side. */
function weightedTarget(prizes: Prize[]): number {
  const total = prizes.reduce((s, p) => s + Math.max(0, p.weight), 0);
  if (total <= 0) return Math.floor(Math.random() * prizes.length);
  let roll = Math.random() * total;
  for (let i = 0; i < prizes.length; i++) {
    roll -= Math.max(0, prizes[i]!.weight);
    if (roll < 0) return i;
  }
  return prizes.length - 1;
}

export function GamePreview({ game, prizes }: { game: string; prizes: Prize[] }) {
  const { title, Component } = gameFor(game);
  const [round, setRound] = useState(0);
  const [target, setTarget] = useState<number | null>(null);
  const [spinning, setSpinning] = useState(false);

  const onSettled = useCallback(() => setSpinning(false), []);

  const segments: Segment[] = prizes
    .filter((p) => p.label.trim() !== "")
    .map((p, i) => ({ id: i + 1, label: p.label }));

  function play() {
    if (spinning || segments.length === 0) return;
    setTarget(weightedTarget(prizes.filter((p) => p.label.trim() !== "")));
    setSpinning(true);
    setRound((r) => r + 1); // remount for a clean replay
  }

  return (
    <div className="preview-stage flex flex-col items-center gap-5 rounded-2xl p-6">
      <div className="flex w-full items-center justify-between">
        <span className="text-sm font-semibold text-white/90">{title}</span>
        <span className="text-xs text-white/40">实时预览</span>
      </div>

      <div className="flex min-h-[17rem] w-full items-center justify-center">
        {segments.length === 0 ? (
          <p className="text-sm text-white/50">先添加奖品，即可预览玩法</p>
        ) : (
          <Component
            key={round}
            segments={segments}
            target={target}
            spinning={spinning}
            onSettled={onSettled}
          />
        )}
      </div>

      <Button
        variant="primary"
        size="md"
        fullWidth
        isDisabled={spinning || segments.length === 0}
        onPress={play}
      >
        {spinning ? "抽奖中…" : "试玩一次"}
      </Button>
    </div>
  );
}
