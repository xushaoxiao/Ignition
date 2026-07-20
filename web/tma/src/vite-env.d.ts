/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** 后端地址。生产走同域反代时留空。 */
  readonly VITE_API_BASE?: string
  // 开发用的 Bot token 走 DEV_BOT_TOKEN（不带 VITE_ 前缀，只有 vite.config
  // 的 Node 侧读得到），因此不出现在这里 —— 它不该被前端代码碰到。
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
