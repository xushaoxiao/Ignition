"use client";

import { Button } from "@heroui/react";
import { gameFor, type Segment } from "@ignition/games";
import { useCallback, useState } from "react";
import { DAILY_BUDGET, DAILY_REWARD_SKIN } from "@/lib/games";
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
  const daily = game === DAILY_BUDGET;
  // The daily game's own code names a decision game, not an animation, so its prize draw is
  // previewed with the skin the TMA actually uses for the reward stage.
  const { title, Component } = gameFor(daily ? DAILY_REWARD_SKIN : game);
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
        <span className="text-sm font-semibold text-white/90">
          {daily ? "每日理财决策" : title}
        </span>
        <span className="text-xs text-white/40">实时预览</span>
      </div>

      {daily && <DailyIntro />}

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

/**
 * Static illustration of the decision round that precedes the draw.
 *
 * Not interactive and not configurable here: the scenario library is platform reference data
 * (`daily_scenario`), maintained with the schema so every campaign asks the same vetted questions
 * and the scoring stays consistent. What a campaign owns is the prize pool below and the optional
 * soft prompt shown to high scorers.
 */
function DailyIntro() {
  return (
    <div className="w-full rounded-2xl bg-white/5 p-4 ring-1 ring-white/10">
      <p className="text-xs text-white/45">今日场景 · 平台内容库</p>
      <p className="mt-1 text-sm font-semibold text-white/90">发工资了</p>
      <p className="mt-1 text-xs leading-relaxed text-white/60">
        本月工资到账，房租水电扣掉后还剩 5,000 元。你第一件事做什么？
      </p>
      <div className="mt-3 grid gap-1.5">
        {["先转 1,500 元进货币基金", "列一份本月开支计划", "先放着，想花就花", "提前还清信用卡"].map(
          (label) => (
            <span
              key={label}
              className="rounded-xl bg-white/5 px-3 py-2 text-xs text-white/70"
            >
              {label}
            </span>
          ),
        )}
      </div>
      <p className="mt-3 text-xs text-white/40">
        选择后即时评分 + 科普，连续打卡加分；完成后进入下方抽奖 ↓
      </p>
    </div>
  );
}
