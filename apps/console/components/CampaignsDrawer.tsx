"use client";

import { Button } from "@heroui/react";
import { gameFor } from "@ignition/games";
import { deepLink } from "@/lib/links";
import type { Campaign } from "@/lib/types";

export function CampaignsDrawer({
  open,
  campaigns,
  onClose,
  onDelete,
}: {
  open: boolean;
  campaigns: Campaign[];
  onClose: () => void;
  onDelete: (id: number) => void;
}) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-40 flex justify-end bg-black/40 backdrop-blur-sm" onClick={onClose}>
      <aside
        className="flex h-full w-full max-w-md flex-col border-l border-black/10 bg-background dark:border-white/10"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-black/10 px-6 py-4 dark:border-white/10">
          <h2 className="text-lg font-semibold">我的活动 ({campaigns.length})</h2>
          <button
            aria-label="关闭"
            onClick={onClose}
            className="grid size-8 place-items-center rounded-lg text-black/50 hover:bg-black/5 dark:text-white/50 dark:hover:bg-white/5"
          >
            ✕
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-4">
          {campaigns.length === 0 ? (
            <p className="mt-10 text-center text-sm text-black/45 dark:text-white/45">
              还没有活动，去左侧配置一个吧。
            </p>
          ) : (
            <ul className="flex flex-col gap-3">
              {campaigns.map((c) => (
                <li
                  key={c.id}
                  className="rounded-2xl border border-black/10 p-4 dark:border-white/10"
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p className="truncate font-semibold">{c.name}</p>
                      <p className="mt-0.5 text-xs text-black/50 dark:text-white/50">
                        {gameFor(c.game).title} · {c.prizes.length} 个奖品 ·{" "}
                        {new Date(c.createdAt).toLocaleDateString("zh-CN")}
                      </p>
                    </div>
                    <button
                      aria-label="删除"
                      onClick={() => onDelete(c.id)}
                      className="shrink-0 rounded-lg px-2 py-1 text-xs text-black/40 hover:bg-red-500/10 hover:text-red-500 dark:text-white/40"
                    >
                      删除
                    </button>
                  </div>
                  <code className="mt-2 block truncate rounded-lg bg-black/[0.04] px-2.5 py-1.5 text-xs dark:bg-white/[0.06]">
                    {deepLink(c)}
                  </code>
                </li>
              ))}
            </ul>
          )}
        </div>

        <div className="border-t border-black/10 p-4 dark:border-white/10">
          <Button variant="outline" size="md" fullWidth onPress={onClose}>
            返回配置
          </Button>
        </div>
      </aside>
    </div>
  );
}
