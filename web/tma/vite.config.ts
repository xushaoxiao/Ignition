import { createHmac } from 'node:crypto'
import { defineConfig, loadEnv, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

/**
 * 开发用的 initData 签发端点。
 *
 * initData 必须由 Bot token 签名，浏览器签不出来。原先的做法是把一串预先签好
 * 的塞进 .env.local —— 但 initData 的时效上限是 5 分钟，那串东西签完就开始腐烂，
 * 隔一会儿回来就是「登录信息无效」，还得重签、重启 dev server。
 *
 * 改成开发服务器现场签：每次请求都是新鲜的，永远不过期。
 *
 * **只在 `vite dev` 里存在**，`vite build` 不会产出这段代码，生产环境不可能带上。
 * 而且 token 从**不带 VITE_ 前缀**的环境变量读 —— 那类变量只有 Node 侧的配置
 * 文件能拿到，不会被打进任何前端 bundle。
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
              error: '需要在 web/tma/.env.local 里配置 DEV_BOT_TOKEN 与 DEV_TRACKING_ID',
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

        // Telegram 官方算法：字段按 key 升序拼成 data_check_string，
        // 密钥是 HMAC(key="WebAppData", data=bot_token)。
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
  // 第三个参数留空前缀：连不带 VITE_ 的变量一起读进来。它们只留在 Node 侧，
  // 不会进 bundle —— DEV_BOT_TOKEN 正是靠这一点不泄漏给浏览器。
  const env = loadEnv(mode, process.cwd(), '')

  return {
    plugins: [react(), tailwindcss(), devInitData(env)],
    server: {
      // Telegram 只加载 HTTPS 页面，本地调试需要把 dev server 暴露给隧道工具
      // （cloudflared / ngrok），所以监听 0.0.0.0 并放开 host 校验。
      host: true,
      allowedHosts: true,
    },
    build: { outDir: 'dist', sourcemap: true },
  }
})
