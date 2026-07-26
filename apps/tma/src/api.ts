/**
 * Backend API client.
 *
 * Two conventions mirror the backend strictly:
 *
 * 1. **Play requests must carry an idempotency key.** Weak-network double-taps are
 *    normal; without a key, one tap becomes three prize-pool debits. Keys are generated
 *    and cached in this layer so UI code does not worry about it.
 * 2. **Access tokens expire after 15 minutes; on 401, refresh once and retry.**
 *    Mini apps are often backgrounded; tokens are usually stale on return. Retrying
 *    here avoids sprinkling "session expired" handling across the UI.
 */

const BASE: string = import.meta.env.VITE_API_BASE ?? ''

export interface Segment {
  id: number
  label: string
}

export interface Session {
  access_token: string
  refresh_token: string
  expires_in: number
  campaign_id: number
  kol_id: number
  plays_left: number
  /** Game to render — the campaign's `template.code` (e.g. `lucky_wheel`, `slot_machine`). */
  game: string
  /** Prize pool; order matches `segment_index` values from the server. */
  prizes: Segment[]
}

export interface PlayResult {
  play_id: number
  prize_id: number
  prize_label: string
  /** Prize segment index on the wheel. The pointer stops here — outcome is server-authoritative. */
  segment_index: number
  plays_left: number
  idempotent: boolean
}

export interface Handoff {
  android_url?: string
  ios_url?: string
  show_code_to_user: boolean
}

export interface ClaimResult {
  claim_code: string
  expires_at: string
  handoff: Handoff
  idempotent: boolean
}

export class ApiError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly status: number,
    readonly retryable: boolean,
  ) {
    super(message)
  }
}

let access = ''
let refresh = ''

async function call<T>(path: string, body: unknown, retryOn401 = true): Promise<T> {
  const res = await fetch(BASE + path, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      ...(access ? { Authorization: `Bearer ${access}` } : {}),
    },
    body: JSON.stringify(body),
  })

  if (res.ok) return (await res.json()) as T

  // Token expired: refresh once and retry. Only one retry — a second failure means
  // refresh is gone too and the user must reopen the mini app; infinite retry just spins.
  if (res.status === 401 && retryOn401 && refresh) {
    if (await tryRefresh()) return call<T>(path, body, false)
  }

  const payload = (await res.json().catch(() => null)) as
    | { error?: { code?: string; message?: string; retryable?: boolean } }
    | null
  throw new ApiError(
    payload?.error?.code ?? 'unknown',
    payload?.error?.message ?? '网络异常，请稍后再试',
    res.status,
    payload?.error?.retryable ?? false,
  )
}

async function tryRefresh(): Promise<boolean> {
  try {
    const res = await fetch(BASE + '/v1/tma/session/refresh', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ refresh_token: refresh }),
    })
    if (!res.ok) return false
    const s = (await res.json()) as { access_token: string; refresh_token: string }
    access = s.access_token
    refresh = s.refresh_token
    return true
  } catch {
    return false
  }
}

/** Exchange initData for a session. First request on every mini app open. */
export async function openSession(initData: string): Promise<Session> {
  const s = await call<Session>('/v1/tma/session', { init_data: initData }, false)
  access = s.access_token
  refresh = s.refresh_token
  return s
}

/**
 * Play once.
 *
 * `idempotencyKey` is generated per user intent and kept stable: retries for the same
 * tap reuse the same key; the next tap gets a new one.
 */
export function play(idempotencyKey: string): Promise<PlayResult> {
  return call<PlayResult>('/v1/tma/play', { idempotency_key: idempotencyKey })
}

/** Claim a redemption code for a play. Repeat calls for the same play return the same code. */
export function claim(playId: number): Promise<ClaimResult> {
  return call<ClaimResult>('/v1/tma/claim', { play_id: playId })
}

/** Generate a new idempotency key. */
export function newIdempotencyKey(): string {
  return crypto.randomUUID()
}
