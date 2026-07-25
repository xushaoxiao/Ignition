/**
 * Claim code card.
 *
 * **This component directly drives iOS revenue.**
 *
 * There is no reliable user-level deferred deep link on iOS, so "user manually enters
 * this code in the main app" is the only billable attribution path on that side. Hard-to-
 * read codes, failed copy, unclear next steps — each is direct revenue loss, not polish.
 *
 * Hence the deliberate extras: monospace grouped display, explicit copy feedback,
 * platform-specific next-step guidance, and expiry shown in plain language.
 */
import { useState } from 'react'
import type { ClaimResult } from '../api'
import { shareInvite } from '../telegram'

interface Props {
  claim: ClaimResult
  prizeLabel: string
}

export function ClaimCard({ claim, prizeLabel }: Props) {
  const [copied, setCopied] = useState(false)
  const ios = isIOS()
  const storeUrl = ios ? claim.handoff.ios_url : claim.handoff.android_url

  async function copy() {
    try {
      await navigator.clipboard.writeText(claim.claim_code)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 2000)
    } catch {
      // Clipboard unavailable in some WebViews.
    }
  }

  function handleShare() {
    shareInvite(`🎁 我刚刚抽中了 ${prizeLabel}！快来幸运转盘一起抽取大奖，我的兑换码是：${claim.claim_code}`)
  }

  return (
    <div className="flex flex-col gap-5 text-center">
      <div>
        <p className="text-sm text-white/60">恭喜获得</p>
        <p className="mt-1 text-2xl font-bold text-amber-300">{prizeLabel}</p>
      </div>

      <div className="rounded-2xl bg-white/10 p-5 ring-1 ring-white/15">
        <p className="text-xs text-white/60">你的兑换码</p>
        <p className="mt-2 select-all font-mono text-3xl font-bold tracking-[0.35em] text-white">
          {claim.claim_code}
        </p>
        <div className="mt-4 flex gap-2">
          <button
            onClick={copy}
            className="flex-1 rounded-xl bg-white/15 py-2.5 text-sm font-medium text-white active:bg-white/25"
          >
            {copied ? '已复制 ✓' : '复制兑换码'}
          </button>
          <button
            onClick={handleShare}
            className="flex-1 rounded-xl bg-emerald-500/80 py-2.5 text-sm font-medium text-white active:bg-emerald-600"
          >
            分享欧气 🚀
          </button>
        </div>
        <p className="mt-3 text-xs text-white/50">
          有效期至 {formatExpiry(claim.expires_at)}，过期需重新抽奖
        </p>
      </div>

      {storeUrl && (
        <a
          href={storeUrl}
          target="_blank"
          rel="noreferrer"
          className="rounded-xl bg-amber-400 py-3.5 text-base font-semibold text-slate-900 active:bg-amber-500"
        >
          {ios ? '前往 App Store 下载' : '前往 Google Play 下载'}
        </a>
      )}

      <ol className="space-y-1.5 text-left text-sm text-white/70">
        <li>1. 下载并打开主 App</li>
        <li>
          2.{' '}
          {ios
            ? '在首页弹窗里粘贴上面的兑换码'
            : '通常会自动识别；若没有，粘贴上面的兑换码'}
        </li>
        <li>3. 奖励立即到账</li>
      </ol>
    </div>
  )
}

/**
 * Rough iOS detection.
 *
 * Misclassification cost is asymmetric: treating iOS as Android makes users wait for
 * auto-recognition that never comes, then churn — a lost conversion. Prefer false iOS;
 * an extra "paste the code" line costs Android users nothing.
 */
function isIOS(): boolean {
  const ua = navigator.userAgent
  if (/iPad|iPhone|iPod/.test(ua)) return true
  // iPadOS 13+ reports Macintosh; use touch points to distinguish.
  return ua.includes('Macintosh') && navigator.maxTouchPoints > 1
}

function formatExpiry(iso: string): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso
  return `${d.getMonth() + 1} 月 ${d.getDate()} 日 ${String(d.getHours()).padStart(2, '0')}:${String(
    d.getMinutes(),
  ).padStart(2, '0')}`
}
