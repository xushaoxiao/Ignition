/**
 * 后端接口客户端。
 *
 * 两条约定与后端严格对应：
 *
 * 1. **抽奖必须带幂等键。** 用户在弱网下狂点是常态，没有幂等键，
 *    一次点击会变成三次扣奖池。键在这一层生成并缓存，业务代码不用操心。
 * 2. **access 令牌 15 分钟到期，401 时用 refresh 换新的并自动重试一次。**
 *    Mini App 常被挂在后台，回来时令牌大概率已经过期；把重试放在这里，
 *    UI 就不需要到处处理「会话过期」。
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
  /** 转盘扇区，顺序即 `segment_index` 的取值顺序，由服务端决定。 */
  prizes: Segment[]
}

export interface PlayResult {
  play_id: number
  prize_id: number
  prize_label: string
  /** 奖项在转盘上的扇区下标。指针停在这里 —— 结果是服务端定的。 */
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

  // 令牌过期：换一次新的再重试。只重试一次 —— 再失败说明 refresh 也没了，
  // 那是真的要请用户重开小程序，无限重试只会把失败拖成转圈。
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

/** 用 initData 换会话。这是每次打开小程序的第一个请求。 */
export async function openSession(initData: string): Promise<Session> {
  const s = await call<Session>('/v1/tma/session', { init_data: initData }, false)
  access = s.access_token
  refresh = s.refresh_token
  return s
}

/**
 * 抽一次奖。
 *
 * `idempotencyKey` 由调用方在「一次用户意图」的粒度上生成并保持不变：
 * 同一次点击的重试要带同一个键，下一次点击才换新的。
 */
export function play(idempotencyKey: string): Promise<PlayResult> {
  return call<PlayResult>('/v1/tma/play', { idempotency_key: idempotencyKey })
}

/** 为一次抽奖领取兑换码。同一次抽奖重复调用会拿到同一个码。 */
export function claim(playId: number): Promise<ClaimResult> {
  return call<ClaimResult>('/v1/tma/claim', { play_id: playId })
}

/** 生成一个幂等键。 */
export function newIdempotencyKey(): string {
  return crypto.randomUUID()
}
