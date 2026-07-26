"use client";

import { Button } from "@heroui/react";
import { useEffect, useMemo, useState } from "react";
import { createCampaign, deleteCampaign, listCampaigns } from "@/lib/mockApi";
import type { Campaign, Prize } from "@/lib/types";
import { CampaignsDrawer } from "./CampaignsDrawer";
import { GamePicker } from "./GamePicker";
import { GamePreview } from "./GamePreview";
import { makePrize, PrizeEditor } from "./PrizeEditor";
import { PublishDialog } from "./PublishDialog";
import { Field, inputClass, Panel, SectionTitle } from "./ui";

function defaultPrizes(): Prize[] {
  return [
    makePrize("100 金币", 60, 500),
    makePrize("500 金币", 25, 100),
    makePrize("限定皮肤", 5, 20),
    makePrize("谢谢参与", 10, 99999),
  ];
}

export function Studio() {
  const [name, setName] = useState("");
  const [game, setGame] = useState("lucky_wheel");
  const [dailyLimit, setDailyLimit] = useState(3);
  const [startsAt, setStartsAt] = useState("");
  const [endsAt, setEndsAt] = useState("");
  const [prizes, setPrizes] = useState<Prize[]>(defaultPrizes);

  const [campaigns, setCampaigns] = useState<Campaign[]>([]);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [published, setPublished] = useState<Campaign | null>(null);

  useEffect(() => setCampaigns(listCampaigns()), []);

  const totalWeight = prizes.reduce((s, p) => s + Math.max(0, p.weight), 0);
  const issues = useMemo(() => {
    const list: string[] = [];
    if (name.trim() === "") list.push("填写活动名称");
    if (prizes.length < 2) list.push("至少需要 2 个奖品");
    if (prizes.some((p) => p.label.trim() === "")) list.push("每个奖品都要有名称");
    if (totalWeight <= 0) list.push("至少一个奖品的权重大于 0");
    if (dailyLimit < 1) list.push("每日抽奖次数至少为 1");
    if (startsAt && endsAt && new Date(startsAt) >= new Date(endsAt))
      list.push("结束时间要晚于开始时间");
    return list;
  }, [name, prizes, totalWeight, dailyLimit, startsAt, endsAt]);
  const valid = issues.length === 0;

  function resetForm() {
    setName("");
    setPrizes(defaultPrizes());
    setStartsAt("");
    setEndsAt("");
  }

  function publish() {
    if (!valid) return;
    const c = createCampaign(
      {
        name: name.trim(),
        game,
        dailyPlayLimit: dailyLimit,
        startsAt: startsAt || null,
        endsAt: endsAt || null,
        prizes,
      },
      new Date().toISOString(),
    );
    setCampaigns(listCampaigns());
    setPublished(c);
  }

  function remove(id: number) {
    deleteCampaign(id);
    setCampaigns(listCampaigns());
  }

  return (
    <div className="min-h-dvh">
      <header className="sticky top-0 z-30 border-b border-black/5 bg-background/80 backdrop-blur-xl dark:border-white/10">
        <div className="mx-auto flex h-16 w-full max-w-7xl items-center justify-between px-6">
          <div className="flex items-center gap-2.5">
            <span className="grid size-8 place-items-center rounded-lg bg-brand font-bold text-white">
              ✦
            </span>
            <div className="leading-tight">
              <p className="text-sm font-semibold">Ignition 活动工作台</p>
              <p className="text-xs text-black/45 dark:text-white/45">配置玩法 · 生成投放页</p>
            </div>
          </div>
          <Button variant="outline" size="sm" onPress={() => setDrawerOpen(true)}>
            我的活动 ({campaigns.length})
          </Button>
        </div>
      </header>

      <main className="mx-auto grid w-full max-w-7xl gap-6 px-6 py-8 lg:grid-cols-[1fr_26rem]">
        {/* Config column */}
        <div className="flex flex-col gap-6">
          <Panel>
            <SectionTitle step={1}>活动基础</SectionTitle>
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="sm:col-span-2">
                <Field label="活动名称">
                  <input
                    className={inputClass}
                    value={name}
                    placeholder="如：双十一每日签到抽奖"
                    onChange={(e) => setName(e.target.value)}
                  />
                </Field>
              </div>
              <Field label="每日抽奖次数" hint="风控 L1：单用户每日上限">
                <input
                  type="number"
                  min={1}
                  className={inputClass}
                  value={dailyLimit}
                  onChange={(e) => setDailyLimit(Math.max(1, Number(e.target.value) || 1))}
                />
              </Field>
              <div className="hidden sm:block" />
              <Field label="开始时间" hint="留空表示立即开始">
                <input
                  type="datetime-local"
                  className={inputClass}
                  value={startsAt}
                  onChange={(e) => setStartsAt(e.target.value)}
                />
              </Field>
              <Field label="结束时间" hint="留空表示长期有效">
                <input
                  type="datetime-local"
                  className={inputClass}
                  value={endsAt}
                  onChange={(e) => setEndsAt(e.target.value)}
                />
              </Field>
            </div>
          </Panel>

          <Panel>
            <SectionTitle step={2}>选择玩法</SectionTitle>
            <GamePicker value={game} onChange={setGame} />
          </Panel>

          <Panel>
            <SectionTitle step={3}>配置奖池</SectionTitle>
            <PrizeEditor prizes={prizes} onChange={setPrizes} />
          </Panel>
        </div>

        {/* Preview + publish column */}
        <div className="flex flex-col gap-4 lg:sticky lg:top-24 lg:self-start">
          <GamePreview game={game} prizes={prizes} />

          <Panel className="!p-5">
            {valid ? (
              <p className="text-sm text-emerald-600 dark:text-emerald-400">✓ 配置完整，可以生成活动</p>
            ) : (
              <div>
                <p className="text-sm font-medium text-black/60 dark:text-white/60">发布前还需：</p>
                <ul className="mt-2 space-y-1">
                  {issues.map((i) => (
                    <li key={i} className="text-sm text-amber-600 dark:text-amber-400">
                      · {i}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            <div className="mt-4">
              <Button variant="primary" size="lg" fullWidth isDisabled={!valid} onPress={publish}>
                生成活动并获取投放链接
              </Button>
            </div>
            <p className="mt-2 text-center text-xs text-black/40 dark:text-white/40">
              演示环境：活动保存在本地浏览器
            </p>
          </Panel>
        </div>
      </main>

      <CampaignsDrawer
        open={drawerOpen}
        campaigns={campaigns}
        onClose={() => setDrawerOpen(false)}
        onDelete={remove}
      />
      <PublishDialog
        campaign={published}
        onClose={() => setPublished(null)}
        onNew={() => {
          setPublished(null);
          resetForm();
        }}
      />
    </div>
  );
}
