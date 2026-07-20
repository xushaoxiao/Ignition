/**
 * 领奖码卡片。
 *
 * **这个组件直接决定 iOS 侧的收入。**
 *
 * iOS 上没有可靠的 user-level deferred deep link，所以「用户把这个码手动输进
 * 主 App」是那一侧唯一的可计费归因路径。码看不清、复制不了、不知道下一步该
 * 干什么 —— 每一样都是直接的收入损失，不是体验瑕疵。
 *
 * 因此这里刻意做了几件在别处显得过度的事：码用等宽大字号分组显示、
 * 复制按钮给明确的成功反馈、按平台给不同的下一步指引、把有效期写出来。
 */
import { useState } from 'react'
import type { ClaimResult } from '../api'

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
      // 剪贴板在部分 WebView 里不可用。码本身是显示出来的，用户仍可手抄 ——
      // 所以这里不弹错误吓唬人。
    }
  }

  return (
    <div className="flex flex-col gap-5 text-center">
      <div>
        <p className="text-sm text-white/60">恭喜获得</p>
        <p className="mt-1 text-2xl font-bold text-amber-300">{prizeLabel}</p>
      </div>

      <div className="rounded-2xl bg-white/10 p-5 ring-1 ring-white/15">
        <p className="text-xs text-white/60">你的兑换码</p>
        {/* tracking-widest + 等宽：8 位码里最容易读错的就是相邻字符，
            字符集已经排除了 0/O/1/I/L，字距再帮一把。 */}
        <p className="mt-2 select-all font-mono text-3xl font-bold tracking-[0.35em] text-white">
          {claim.claim_code}
        </p>
        <button
          onClick={copy}
          className="mt-4 w-full rounded-xl bg-white/15 py-2.5 text-sm font-medium text-white active:bg-white/25"
        >
          {copied ? '已复制 ✓' : '复制兑换码'}
        </button>
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
 * 粗略判断 iOS。
 *
 * 判错的代价是不对称的：把 iOS 当成 Android，用户会等一个永远不会发生的
 * 自动识别，然后放弃 —— 那是一笔丢掉的转化。所以宁可多判成 iOS，
 * 多显示一句「粘贴兑换码」对 Android 用户没有损失。
 */
function isIOS(): boolean {
  const ua = navigator.userAgent
  if (/iPad|iPhone|iPod/.test(ua)) return true
  // iPadOS 13+ 默认上报 Macintosh，靠触摸点数区分。
  return ua.includes('Macintosh') && navigator.maxTouchPoints > 1
}

function formatExpiry(iso: string): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso
  return `${d.getMonth() + 1} 月 ${d.getDate()} 日 ${String(d.getHours()).padStart(2, '0')}:${String(
    d.getMinutes(),
  ).padStart(2, '0')}`
}
