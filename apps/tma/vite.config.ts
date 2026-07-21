import { createHmac } from 'node:crypto'
import { defineConfig, loadEnv, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

/**
 * Dev-only initData signing endpoint.
 *
 * initData must be signed with the bot token; the browser cannot do that. The old
 * approach was to pre-sign a string and put it in .env.local — but initData expires
 * after 5 minutes, so that string rots and you get "invalid login" until you re-sign
 * and restart the dev server.
 *
 * The dev server signs on each request instead: always fresh, never expired.
 *
 * **Exists only in `vite dev`**; `vite build` never ships this code, so production
 * cannot include it. The token is read from env vars **without the VITE_ prefix** —
 * those are only available to Node-side config and are never bundled into the frontend.
 */
function devInitData(env: Record<string, string>): Plugin {
  return {
    name: 'ignition-dev-init-data',
    apply: 'serve',
    configureServer(server) {
      server.middlewares.use('/__dev/init-data', (_req, res) => {
        const token = env.DEV_BOT_TOKEN
        const trackingId = env.DEV_TRACKING_ID
        res.setHeader('Content-Type', 'application/json')
        if (!token || !trackingId) {
          res.statusCode = 503
          res.end(
            JSON.stringify({
              error: 'Set DEV_BOT_TOKEN and DEV_TRACKING_ID in apps/tma/.env.local',
            }),
          )
          return
        }

        const fields: Record<string, string> = {
          auth_date: String(Math.floor(Date.now() / 1000)),
          start_param: trackingId,
          user: JSON.stringify({
            id: Number(env.DEV_TG_USER_ID ?? 424242),
            first_name: 'Dev',
            username: 'dev_user',
          }),
        }

        // Telegram official algorithm: fields sorted by key into data_check_string;
        // secret key is HMAC(key="WebAppData", data=bot_token).
        const dcs = Object.keys(fields)
          .sort()
          .map((k) => `${k}=${fields[k]}`)
          .join('\n')
        const secret = createHmac('sha256', 'WebAppData').update(token).digest()
        const hash = createHmac('sha256', secret).update(dcs).digest('hex')

        const raw = Object.entries(fields)
          .map(([k, v]) => `${k}=${encodeURIComponent(v)}`)
          .concat(`hash=${hash}`)
          .join('&')
        res.end(JSON.stringify({ init_data: raw }))
      })
    },
  }
}

export default defineConfig(({ mode }) => {
  // Empty prefix on the third arg: load VITE_-less vars too. They stay on the Node
  // side and never enter the bundle — DEV_BOT_TOKEN relies on that to stay out of the browser.
  const env = loadEnv(mode, process.cwd(), '')

  return {
    plugins: [react(), tailwindcss(), devInitData(env)],
    server: {
      // Telegram only loads HTTPS; local debugging exposes the dev server via a
      // tunnel (cloudflared / ngrok), so listen on 0.0.0.0 and relax host checks.
      host: true,
      allowedHosts: true,
    },
    build: { outDir: 'dist', sourcemap: true },
  }
})
