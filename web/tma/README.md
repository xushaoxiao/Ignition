# Ignition TMA

Telegram Mini App 的转盘前端。React + Vite + Tailwind。

这一层是 `ChannelAdapter` 扩展点的第一个实例 —— 用它来验证抽象是否成立，
而不是先写抽象再写实现。将来接 Discord Activities 时，换掉的是
[src/telegram.ts](src/telegram.ts)，不是转盘。

## 两条不能改的规则

### 抽奖结果由服务端产生，前端只播动画

`POST /v1/tma/play` 返回 `segment_index`，前端据此**反推**该转到哪个角度。
不是先转、转完再问结果。顺序反过来的话，中奖概率就落在客户端手里 ——
奖池成本和后续那条可计费转化都跟着不可信。

### initData 必须原样上传

签名是对原始字段序列算的。前端一旦解析再拼回去，键序或转义只要差一点，
服务端验签就会失败，表现为「有些人打不开」——这类问题极难排查。

## 领奖码那一屏值得反复打磨

iOS 上没有可靠的 user-level deferred deep link，所以「用户把码手动输进主 App」
是那一侧**唯一**的可计费归因路径。码看不清、复制不了、不知道下一步该干什么，
每一样都是直接的收入损失，不是体验瑕疵。

[src/components/ClaimCard.tsx](src/components/ClaimCard.tsx) 里那些看起来过度的
细节（等宽大字号分组、明确的复制反馈、按平台给不同指引）都源于这一条。
W7 种子内测要验证的核心指标就是 **iOS 领奖码核销完成率 > 40%**。

## 本地开发

```bash
pnpm install
cp .env.example .env.local
pnpm dev
```

Telegram 只加载 HTTPS 页面，真机调试需要隧道：

```bash
cloudflared tunnel --url http://localhost:5173
# 把 https 地址填进 @BotFather 的 Mini App 设置
```

### 不起隧道，在浏览器里跑通全流程

`pnpm dev` 会挂一个 `/__dev/init-data` 端点，用 Bot token 现场签发 initData，
前端在非 Telegram 环境下自动回落到它。**每次请求都是新鲜的**——
initData 的时效上限是 5 分钟，写死在 .env 里的那串签完就开始腐烂。

在 `.env.local` 里配两项：

```bash
DEV_BOT_TOKEN=123456:AA-demo-bot-token   # 与库里 bot.token_enc 存的一致
DEV_TRACKING_ID=aB3xY9zK1m
```

**不带 `VITE_` 前缀是刻意的**：那类变量只有 `vite.config.ts` 的 Node 侧读得到，
不会进任何前端 bundle。签名这件事也整个发生在 dev server 里，
`apply: 'serve'` 决定了 `vite build` 根本不会产出这段代码。

后端还需要放行本地来源，在 `configs/config.yaml` 里：

```yaml
http:
  cors_origins: ["http://localhost:5173"]
```

用允许列表而不是 `*`：这些接口带 Bearer 令牌。

## 构建

```bash
pnpm build      # tsc --noEmit && vite build
```

产物是纯静态文件，`dist/` 直接丢 CDN。生产环境把 API 反代到同域即可省掉 CORS。
