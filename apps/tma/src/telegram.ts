/**
 * Telegram Mini App runtime.
 *
 * This layer does three things only: initialise the SDK, retrieve raw initData, and
 * wrap haptic feedback. Business logic must not touch the SDK directly — when
 * `ChannelAdapter` adds Discord Activities, swap this file, not the wheel.
 */
import {
  hapticFeedbackImpactOccurred,
  hapticFeedbackNotificationOccurred,
  init,
  isTMA,
  expandViewport,
  mountViewport,
  retrieveRawInitData,
} from '@telegram-apps/sdk'

let ready = false

/** True when running inside the Telegram client; false when opened directly in a browser. */
export const inTelegram = (): boolean => {
  try {
    return isTMA()
  } catch {
    return false
  }
}

/** Initialise the SDK and expand the mini app to full height. Failures are swallowed — missing capabilities must not white-screen. */
export async function setupTelegram(): Promise<void> {
  if (ready || !inTelegram()) return
  ready = true
  try {
    init()
    if (mountViewport.isAvailable()) await mountViewport()
    if (expandViewport.isAvailable()) expandViewport()
  } catch {
    // Older clients may not support some methods. The wheel does not depend on them; degrade silently.
  }
}

/**
 * Retrieve the raw initData string.
 *
 * **Must be passed to the backend verbatim.** The signature is over the raw field
 * sequence; parse-and-reserialise on the frontend breaks verification — showing up as
 * "some users cannot open the app".
 *
 * Outside Telegram (`pnpm dev` in a browser), fall back to the dev server's
 * `/__dev/init-data` on-the-fly signer. **That endpoint exists only under `vite dev`**;
 * production builds have no such route, and this fetch is tree-shaken via `import.meta.env.DEV`.
 *
 * On-the-fly signing instead of a pre-baked string: initData expires after 5 minutes;
 * a string baked into .env rots and returns "invalid login" after a short break.
 */
export async function rawInitData(): Promise<string | null> {
  try {
    const real = retrieveRawInitData()
    if (real) return real
  } catch {
    // Not in Telegram; use dev fallback below
  }

  if (!import.meta.env.DEV) return null
  try {
    const res = await fetch('/__dev/init-data')
    if (!res.ok) return null
    const body = (await res.json()) as { init_data?: string }
    return body.init_data ?? null
  } catch {
    return null
  }
}

/** Light haptic on spin start. Skipped silently when unavailable. */
export function tapFeedback(): void {
  try {
    if (hapticFeedbackImpactOccurred.isAvailable()) {
      hapticFeedbackImpactOccurred('medium')
    }
  } catch {
    /* Haptics are nice-to-have; any failure must not affect the spin */
  }
}

/** Share viral invitation link or prize claim in Telegram chat/groups. */
export function shareInvite(text: string, url?: string): void {
  const targetUrl = url ?? window.location.href
  const shareUrl = `https://t.me/share/url?url=${encodeURIComponent(targetUrl)}&text=${encodeURIComponent(text)}`
  try {
    const tg = (window as unknown as { Telegram?: { WebApp?: { openTelegramLink?: (url: string) => void } } }).Telegram
    if (inTelegram() && tg?.WebApp?.openTelegramLink) {
      tg.WebApp.openTelegramLink(shareUrl)
    } else {
      window.open(shareUrl, '_blank')
    }
  } catch {
    window.open(shareUrl, '_blank')
  }
}

/** Success haptic on win. */
export function successFeedback(): void {
  try {
    if (hapticFeedbackNotificationOccurred.isAvailable()) {
      hapticFeedbackNotificationOccurred('success')
    }
  } catch {
    /* Same as above */
  }
}
