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

async function request<T>(
  method: 'GET' | 'POST',
  path: string,
  body?: unknown,
  retryOn401 = true,
): Promise<T> {
  const res = await fetch(BASE + path, {
    method,
    headers: {
      ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
      ...(access ? { Authorization: `Bearer ${access}` } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  })

  if (res.ok) return (await res.json()) as T

  // Token expired: refresh once and retry. Only one retry — a second failure means
  // refresh is gone too and the user must reopen the mini app; infinite retry just spins.
  if (res.status === 401 && retryOn401 && refresh) {
    if (await tryRefresh()) return request<T>(method, path, body, false)
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
  const s = await request<Session>('POST', '/v1/tma/session', { init_data: initData }, false)
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
  return request<PlayResult>('POST', '/v1/tma/play', { idempotency_key: idempotencyKey })
}

/** Claim a redemption code for a play. Repeat calls for the same play return the same code. */
export function claim(playId: number): Promise<ClaimResult> {
  return request<ClaimResult>('POST', '/v1/tma/claim', { play_id: playId })
}

// ---------------------------------------------------------------- daily budget game

export type Grade = 'building' | 'steady' | 'strong' | 'excellent'

/**
 * One answer option.
 *
 * Key and label only — the score, the verdict, and the teaching line stay on the server until
 * the answer is submitted. If the client could read the scores, the game would be a lookup table.
 */
export interface DailyChoice {
  key: string
  label: string
}

export interface DailyScenario {
  id: number
  code: string
  title: string
  prompt: string
  choices: DailyChoice[]
}

/** Result of one decision. Produced by the server; the client only renders it. */
export interface DailyOutcome {
  choice_key: string
  choice_label: string
  /** The decision's own score change. */
  delta: number
  /** Extra points for the check-in streak, kept separate so the UI can credit the habit. */
  streak_bonus: number
  credit: number
  grade: Grade
  grade_label: string
  streak: number
  verdict: string
  tip: string
}

/** Soft prompt configured on the campaign; only present once the player scores high enough. */
export interface Promo {
  text: string
  url?: string
  min_credit: number
}

export interface DailyToday {
  date: string
  scenario: DailyScenario
  credit: number
  grade: Grade
  grade_label: string
  streak: number
  rounds_played: number
  /** Non-null when today's decision is already made — the same shape an answer returns. */
  answered: DailyOutcome | null
  rank: number | null
  players: number
  promo: Promo | null
}

export interface DailyAnswer extends DailyOutcome {
  rank: number
  players: number
  promo: Promo | null
  /** The answer was already recorded; this response repeats it. */
  idempotent: boolean
}

export interface BoardEntry {
  rank: number
  name: string
  credit: number
  streak: number
  me: boolean
}

export interface DailyLeaderboard {
  entries: BoardEntry[]
  my_rank: number | null
  players: number
}

/** Today's scenario plus the player's standing. Safe to call repeatedly. */
export function dailyToday(): Promise<DailyToday> {
  return request<DailyToday>('GET', '/v1/tma/daily')
}

/**
 * Submit today's decision.
 *
 * No idempotency key: the day itself is the key. The server accepts one round per player per day
 * and replays the stored outcome for any repeat, so a retry after a dropped connection is safe.
 */
export function dailyAnswer(choiceKey: string): Promise<DailyAnswer> {
  return request<DailyAnswer>('POST', '/v1/tma/daily/answer', { choice_key: choiceKey })
}

export function dailyLeaderboard(): Promise<DailyLeaderboard> {
  return request<DailyLeaderboard>('GET', '/v1/tma/daily/leaderboard')
}

/** Generate a new idempotency key. */
export function newIdempotencyKey(): string {
  return crypto.randomUUID()
}
