/**
 * Telegram Mini App 运行环境。
 *
 * 这一层只做三件事：初始化 SDK、取出原始 initData、包一层触感反馈。
 * 业务逻辑不直接碰 SDK —— 将来 `ChannelAdapter` 要接 Discord Activities 时，
 * 换掉的是这个文件，不是转盘。
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

/** 在 Telegram 客户端里运行时为 true。浏览器里直接打开则为 false。 */
export const inTelegram = (): boolean => {
  try {
    return isTMA()
  } catch {
    return false
  }
}

/** 初始化 SDK 并把小程序展开到全高。失败不抛错：环境能力缺失不该白屏。 */
export async function setupTelegram(): Promise<void> {
  if (ready || !inTelegram()) return
  ready = true
  try {
    init()
    if (mountViewport.isAvailable()) await mountViewport()
    if (expandViewport.isAvailable()) expandViewport()
  } catch {
    // 老版本客户端可能不支持部分方法。转盘本身不依赖它们，静默降级。
  }
}

/**
 * 取出原始 initData 字符串。
 *
 * **必须原样传给后端**。签名是对原始字段序列算的，前端一旦解析再拼回去，
 * 键序或转义只要差一点，服务端验签就会失败 —— 表现为「有些人打不开」。
 *
 * 不在 Telegram 里时（`pnpm dev` 开浏览器调试），回落到开发服务器的
 * `/__dev/init-data` 现场签一份。**那个端点只在 `vite dev` 下存在**，
 * 生产构建里既没有这个路由，这段 fetch 也会被 `import.meta.env.DEV` 摇掉。
 *
 * 之所以要现签而不是预先塞一串：initData 的时效上限是 5 分钟，
 * 写死在 .env 里的那串签完就开始腐烂，隔一会儿回来就是「登录信息无效」。
 */
export async function rawInitData(): Promise<string | null> {
  try {
    const real = retrieveRawInitData()
    if (real) return real
  } catch {
    // 不在 Telegram 环境里，走下面的开发回落
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

/** 转盘启动时的一下轻震。能力不可用时静默跳过。 */
export function tapFeedback(): void {
  try {
    if (hapticFeedbackImpactOccurred.isAvailable()) {
      hapticFeedbackImpactOccurred('medium')
    }
  } catch {
    /* 触感是锦上添花，任何失败都不该影响抽奖 */
  }
}

/** 中奖时的成功反馈。 */
export function successFeedback(): void {
  try {
    if (hapticFeedbackNotificationOccurred.isAvailable()) {
      hapticFeedbackNotificationOccurred('success')
    }
  } catch {
    /* 同上 */
  }
}
