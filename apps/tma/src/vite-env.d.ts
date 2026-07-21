/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Backend base URL. Leave empty in production when the API is same-origin reverse-proxied. */
  readonly VITE_API_BASE?: string
  // Dev bot token uses DEV_BOT_TOKEN (no VITE_ prefix; only vite.config Node side reads it),
  // so it does not appear here — frontend code must not touch it.
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
