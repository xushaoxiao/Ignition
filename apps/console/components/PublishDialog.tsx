"use client";

import { Button } from "@heroui/react";
import QRCode from "qrcode";
import { useEffect, useState } from "react";
import { deepLink, previewLink } from "@/lib/links";
import type { Campaign } from "@/lib/types";

function CopyRow({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard may be blocked; the value is visible to select manually */
    }
  }
  return (
    <div>
      <span className="mb-1 block text-xs font-medium text-black/50 dark:text-white/50">{label}</span>
      <div className="flex items-center gap-2">
        <code className="flex-1 truncate rounded-lg bg-black/[0.04] px-3 py-2 text-xs dark:bg-white/[0.06]">
          {value}
        </code>
        <Button variant="outline" size="sm" onPress={copy}>
          {copied ? "已复制" : "复制"}
        </Button>
      </div>
    </div>
  );
}

export function PublishDialog({
  campaign,
  onClose,
  onNew,
}: {
  campaign: Campaign | null;
  onClose: () => void;
  onNew: () => void;
}) {
  const [qr, setQr] = useState<string | null>(null);
  const link = campaign ? deepLink(campaign) : "";

  useEffect(() => {
    if (!campaign) return;
    let alive = true;
    QRCode.toDataURL(link, { margin: 1, width: 320 })
      .then((url) => alive && setQr(url))
      .catch(() => alive && setQr(null));
    return () => {
      alive = false;
    };
  }, [campaign, link]);

  if (!campaign) return null;

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-black/50 p-4 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="w-full max-w-lg overflow-hidden rounded-3xl border border-black/10 bg-background shadow-2xl dark:border-white/15"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="border-b border-black/10 bg-brand/5 px-6 py-5 dark:border-white/10">
          <div className="flex items-center gap-2 text-brand">
            <span className="grid size-7 place-items-center rounded-full bg-brand text-sm text-white">✓</span>
            <h2 className="text-lg font-semibold text-foreground">活动已生成</h2>
          </div>
          <p className="mt-1 text-sm text-black/55 dark:text-white/55">
            「{campaign.name}」已就绪 — 扫码或分享链接即可投放。
          </p>
        </div>

        <div className="grid gap-5 p-6 sm:grid-cols-[auto_1fr]">
          <div className="mx-auto w-40 shrink-0">
            {qr ? (
              // eslint-disable-next-line @next/next/no-img-element
              <img
                src={qr}
                alt="投放二维码"
                className="size-40 rounded-xl bg-white p-1.5 ring-1 ring-black/10 dark:ring-white/10"
              />
            ) : (
              <div className="size-40 animate-pulse rounded-xl bg-black/10 dark:bg-white/10" />
            )}
          </div>
          <div className="flex flex-col justify-center gap-3">
            <CopyRow label="投放链接（Telegram 深链）" value={link} />
            <CopyRow label="Tracking ID" value={campaign.trackingId} />
            <a
              href={previewLink(campaign)}
              target="_blank"
              rel="noopener noreferrer"
              className="text-sm font-medium text-brand hover:underline"
            >
              在 Mini App 中预览 ↗
            </a>
          </div>
        </div>

        <div className="flex justify-end gap-2 border-t border-black/10 px-6 py-4 dark:border-white/10">
          <Button variant="outline" size="md" onPress={onNew}>
            再建一个
          </Button>
          <Button variant="primary" size="md" onPress={onClose}>
            完成
          </Button>
        </div>
      </div>
    </div>
  );
}
