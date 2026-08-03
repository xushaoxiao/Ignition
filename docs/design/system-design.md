# 私域游戏化增长引擎 —— 系统设计方案 v1

> 目标读者：技术负责人 / 首批工程团队
> 范围：支撑「平台费 + 效果分成 + 增值」三段式商业模型的完整系统设计
>
> **这份文档描述的是目标状态，不是当前状态。** 已实现到什么程度见仓库根目录
> `README.md`；对外归因口径见 `docs/product/attribution-policy-v1.md`。两者冲突时
> 以 README / 对外政策为准，并回来修正本文。
>
> ### 当前落地摘要（2026-07）
>
> **已跑通**：TMA 开场 → 服务端权威抽奖 → 领奖码签发/核销（`deterministic_code`）→
> 归因 + 可计费事件 → 冷静期放行 → 月末结算（发票 + 复式分录）→ 账本校验；
> API Key HMAC、密钥加密存储、JWT 会话、entitlement 模型、L1/L2 风控采集、
> 变现回传（分析流/计费流分离）。
>
> **已交付（续）**：归因查询 API（`GET /v1/attribution/{app_user_id}`，只读投影，
> 不含 evidence）；信封加密框架（secrets.rs 的 V1/V2 版本分派 + `KeyProvider` 接缝 +
> 本地 provider，真实云 KMS 为部署时接入的薄适配层）；发票推送链路（`job push-invoices`
> 经 `PaymentGateway` 把 draft 发票发给收款方，含 log 网关，按发票幂等，成功后 draft→open，
> 真实 Stripe 适配器为部署时接入层）；`billing.performance` 门控（核销侧按权益决定是否入计费流）。
>
> **尚未交付（相对本文目标态）**：真实 Stripe 适配器与订阅侧 webhook、KOL/客户控制台与
> 三指标看板、ClickHouse 分析流、明细导出/差异视图/申诉通道、冲正触发接口、L3 渠道扫描、
> 真实云 KMS 适配器、其余 entitlement 门控点。Event Bus 仍以 DB 约束承担幂等，
> Redis 已在 compose 就位但代码未用。
>
> 里程碑原文（§12）保留作规划参考；上表以 README「Current status」为准。

---

## 0. 设计目标与硬约束

本方案的所有技术决策，都服务于三段式定价模型带来的四条硬约束：

| # | 约束 | 来源 | 系统含义 |
|---|---|---|---|
| C1 | **计费必须只依赖确定性归因** | 中层只对确定性转化收费 | 概率归因的转化可以进看板，但绝不能进账单。两条数据流必须物理分离 |
| C2 | **归因数据不能有利益冲突** | 底层平台费的存在意义 | 归因判定逻辑必须版本化、可公示、可复算。客户能自己验算出同样的结果 |
| C3 | **计费必须可冲正** | 退款 / 拒付 / 事后判定作弊 | 账本用复式记账，不允许 UPDATE 金额，只允许写反向分录 |
| C4 | **能力门控必须是数据驱动的** | 上层增值按档位解锁 | 不允许 `if plan == 'pro'` 散落在代码里，用 entitlement 表统一驱动 |

另有两条通用约束：

- **C5 多租户强隔离**：一次漏写 `tenant_id` 条件就是数据事故，必须有代码之外的兜底。
- **C6 幂等**：所有外部入口（TG webhook、主 App 回传、支付 webhook）都会重复投递，无一例外。

---

## 1. 商业模型 → 系统能力映射

这一节是整个设计的索引，先建立对应关系，后面各节展开。

```
┌─ 底层：平台费 $99–299/月 ────────────────────────────────┐
│  需要：订阅管理 · 试用期 · 席位/群组配额 · 支付集成       │
│  对应：§8 订阅与门控  ·  §9 计费系统（订阅侧）            │
└──────────────────────────────────────────────────────────┘
┌─ 中层：效果分成（CPA 起步，封顶）────────────────────────┐
│  需要：确定性归因 · 可计费事件定义 · 冲正 · 封顶 · 对账    │
│  对应：§5 归因链路 · §6 计费事件 · §7 账本 · §9 计费（用量侧）│
└──────────────────────────────────────────────────────────┘
┌─ 上层：增值（Discord / 深度分析 / 白标 / KOL 撮合）───────┐
│  需要：能力门控 · 模块化扩展点 · 数据分层                 │
│  对应：§8 entitlement · §3 架构的扩展点设计               │
└──────────────────────────────────────────────────────────┘
```

**贯穿全局的一条原则**：系统内部存在两条数据流，永不混用。

| | 分析流（Analytics） | 计费流（Billing） |
|---|---|---|
| 存储 | ClickHouse | PostgreSQL |
| 一致性 | 最终一致，允许丢失/延迟/近似 | 强一致，事务，零丢失 |
| 数据来源 | 全部事件，含概率归因 | 仅确定性归因 + 风控放行 |
| 用途 | KOL 看板、漏斗分析、趋势 | 账单、分润、对账、申诉 |
| 可否修改 | 可重算 | 只可追加，不可修改 |

看板上要明确标注两个数字：「归因转化（全部）」和「可结算转化」。**主动把差异暴露给客户，比让客户自己发现差异更能建立信任。**

---

## 2. 总体架构

```
                          终端用户 (Telegram)
                                 │
                    ┌────────────▼────────────┐
                    │  TMA 前端 (React+Vite)   │
                    └────────────┬────────────┘
   KOL / 客户                    │ HTTPS + JWT
       │                         │
┌──────▼──────┐          ┌───────▼────────────────────────────┐
│ Console     │          │  API Gateway                       │
│ (Next.js)   ├─────────►│  initData校验 · JWT · 限流 · 幂等键 │
└─────────────┘          └───────┬────────────────────────────┘
                                 │
        ┌────────────────────────┼────────────────────────┐
        │                        │                        │
┌───────▼──────┐  ┌──────────────▼─────┐  ┌───────────────▼────┐
│ Campaign &   │  │ Attribution Svc    │  │ Billing & Ledger   │
│ Game Svc     │  │ (归因判定/领奖码)   │  │ (可计费事件/账本)   │
└───────┬──────┘  └──────────┬─────────┘  └───────────┬────────┘
        │                    │                        │
        │         ┌──────────▼─────────┐              │
        │         │ Risk Engine        │              │
        │         │ L1同步 / L2采集     │──────────────┤ (放行/暂缓)
        │         │ L3异步扫描          │              │
        │         └────────────────────┘              │
        │                                             │
┌───────▼─────────────────────────────────────────────▼────────┐
│  Event Bus (Redis Stream → Kafka)                            │
└───────┬──────────────────────────────────┬───────────────────┘
        │                                  │
┌───────▼────────┐              ┌──────────▼──────────┐
│ PostgreSQL     │              │ ClickHouse          │
│ 业务状态 + 账本 │              │ 事件明细 + 物化视图  │
│ (强一致/权威)   │              │ (分析/看板)         │
└────────────────┘              └─────────────────────┘
        ▲                                  ▲
        │ S2S Postback                     │ Webhook
   客户主 App / MMP                    Telegram Bot API
        ▲
        │
   Stripe (订阅 + 用量计费)
```

### 部署形态

MVP **不拆微服务**。上图的 Svc 是单体内的模块边界（独立 package、只通过接口互调、不共享内部结构体）。只有两个组件独立部署：

1. **Event Ingest**：流量特征与业务服务完全不同（高 QPS、可丢、抗突发），且需要独立扩缩容。
2. **Risk Scanner（L3）**：批处理型，跑重查询，不能影响在线链路。

理由：MVP 阶段最大的风险是交付速度，不是架构优雅度。模块边界画清楚，将来拆是一天的事；一开始就拆，联调和运维成本会吃掉一半工期。

### 扩展点设计（服务于上层增值）

以下四处从第一天就留接口，因为它们是增值功能的挂载点：

| 扩展点 | 接口 | 未来承载 |
|---|---|---|
| `ChannelAdapter` | 分发链接生成 / 身份校验 / 消息推送 | Discord Activities、WhatsApp Flows |
| `TemplateEngine` | 配置 schema / 结算逻辑 / 前端 bundle | 专属模板、白标模板 |
| `AttributionMethod` | 签发 / 核销 / 置信度 | 新的确定性通道 |
| `AnalyticsView` | 查询定义 / 权限等级 | 深度分层分析（付费档） |

TMA 的实现本身就是 `ChannelAdapter` 的第一个实例——**用它验证抽象是否成立**，而不是先写抽象再写实现。

---

## 3. 领域模型

```
Tenant ─────┬── Subscription ── Plan ── Entitlement[]
            │
            ├── App ── { bundle_id, package_name, store_url,
            │            postback_secret, attribution_policy_version }
            ├── Bot ── { encrypted_token, username }
            │
            ├── KOL ─── Channel[] ─── Link[]
            │                          └─ tracking_id (不可枚举)
            ├── Campaign ── Template
            │      └── RewardPool ── RewardItem[]
            │
            ├── Player ── { tg_user_id, app_user_id?, device_ids[] }
            │      └── Attribution
            │
            ├── ClaimCode ── (Player × Campaign × Link)
            │
            ├── BillableEvent ──┬── LedgerEntry[]  (复式，成对)
            │                   └── RiskVerdict
            │
            └── Invoice ── InvoiceLine[]
```

### 关键实体定义

#### `Attribution` — 归因记录（系统的信任基石）

```sql
CREATE TABLE attribution (
  id              BIGSERIAL PRIMARY KEY,
  tenant_id       BIGINT NOT NULL,
  player_id       BIGINT NOT NULL,
  kol_id          BIGINT NOT NULL,
  campaign_id     BIGINT NOT NULL,
  link_id         BIGINT NOT NULL,

  method          attribution_method NOT NULL,  -- 枚举见下
  confidence      SMALLINT NOT NULL,            -- 100 / 100 / 60 / 30
  is_billable     BOOLEAN NOT NULL,             -- 冗余但必要：计费查询不依赖 method 的解释

  policy_version  TEXT NOT NULL,                -- 归因规则版本，用于申诉复算
  touch_at        TIMESTAMPTZ NOT NULL,         -- 首次触点
  attributed_at   TIMESTAMPTZ NOT NULL,         -- 归因成立时刻
  evidence        JSONB NOT NULL,               -- 判定依据快照，只增不改

  UNIQUE (tenant_id, player_id)                 -- 单归因：一个用户只归一个 KOL
);

CREATE TYPE attribution_method AS ENUM (
  'deterministic_code',   -- 领奖码核销      confidence=100  billable=true
  'install_referrer',     -- Play Install Referrer  100      true
  'universal_link',       -- 已安装用户直接唤起      100      true
  'clipboard_match',      -- 剪贴板匹配              60       false
  'probabilistic'         -- 指纹/时间窗匹配         30       false
);
```

**设计要点**：

- `is_billable` 是**冗余字段**，逻辑上可由 `method` 推导。但仍然要存——因为计费规则会变（比如将来把 `clipboard_match` 提为可计费），而已开出的账单必须保持当时的判定。**账单一旦生成，其依据必须冻结。**
- `evidence` 存判定当时的完整输入快照（code、referrer 原文、时间戳、device_id 等）。KOL 申诉时，这是唯一的证据来源。**只增不改，不允许后续补写。**
- `policy_version` 对应一份公开文档。规则变更 = 发新版本 + 提前通知客户，绝不静默修改。这是 C2 的落地方式。
- `UNIQUE(tenant_id, player_id)` 强制单归因。**多归因（分成拆分）在 MVP 阶段明确不做**——它会让对账复杂度指数上升，而 KOL 侧的收益感知反而变差。

#### `BillableEvent` — 可计费事件（收入的原子）

```sql
CREATE TABLE billable_event (
  id              BIGSERIAL PRIMARY KEY,
  tenant_id       BIGINT NOT NULL,
  attribution_id  BIGINT NOT NULL,
  event_type      TEXT NOT NULL,        -- 'activation' | 'iap_purchase' | ...
  external_id     TEXT NOT NULL,        -- 主 App 侧唯一 ID，幂等键

  status          billable_status NOT NULL DEFAULT 'pending',
  amount_cents    BIGINT NOT NULL,      -- 应计费金额（CPA 为固定单价）
  currency        CHAR(3) NOT NULL,

  occurred_at     TIMESTAMPTZ NOT NULL, -- 业务发生时间（主 App 报的）
  received_at     TIMESTAMPTZ NOT NULL, -- 我方接收时间
  hold_until      TIMESTAMPTZ NOT NULL, -- 冷静期结束
  cleared_at      TIMESTAMPTZ,
  billed_at       TIMESTAMPTZ,
  invoice_id      BIGINT,

  UNIQUE (tenant_id, event_type, external_id)   -- 幂等的物理保证
);

CREATE TYPE billable_status AS ENUM (
  'pending',      -- 已接收，在 hold 期内
  'held',         -- 风控暂缓，等待人工
  'cleared',      -- 已放行，可计入账单
  'billed',       -- 已开票
  'reversed',     -- 已冲正（退款/判定作弊）
  'rejected'      -- 判定无效，不计费
);
```

**状态机**：

```
                    ┌──────────► rejected  (归因无效 / 重复 / 超窗)
                    │
  received ──► pending ──► cleared ──► billed
                    │         ▲          │
                    ▼         │          ▼
                  held ───────┘      reversed
                (风控暂缓)          (退款/事后作弊)
                    │
                    └──────────► rejected
```

**关键规则**：

1. 只有 `attribution.is_billable = true` 的事件才会创建 `BillableEvent`。不可计费的转化只进 ClickHouse。
2. `hold_until` 默认 `occurred_at + 7 天`（CPA）/ `+ 35 天`（未来的 GMV 分成，覆盖 App Store 退款窗口）。**hold 期内的事件在看板上显示为「待确认」，不是「已确认」**——不要给客户先高后低的数字体验。
3. `billed` 之后仍可 `reversed`，冲正走下个账期的信用额度（credit note），不追溯改已出账单。

#### `LedgerEntry` — 复式记账

```sql
CREATE TABLE ledger_entry (
  id            BIGSERIAL PRIMARY KEY,
  tenant_id     BIGINT NOT NULL,
  txn_id        UUID NOT NULL,          -- 同一笔交易的分录共享
  account       TEXT NOT NULL,          -- 科目
  direction     CHAR(1) NOT NULL,       -- 'D' | 'C'
  amount_cents  BIGINT NOT NULL CHECK (amount_cents > 0),
  currency      CHAR(3) NOT NULL,
  ref_type      TEXT NOT NULL,          -- 'billable_event' | 'subscription' | 'reversal'
  ref_id        BIGINT NOT NULL,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- 无 UPDATE 权限，无 DELETE 权限（数据库角色层面强制）
```

科目表（MVP 够用）：

| 科目 | 含义 |
|---|---|
| `tenant_receivable` | 客户应付我方 |
| `platform_revenue` | 平台收入 |
| `kol_payable` | 我方应付 KOL（若走代付模式） |
| `reversal_clearing` | 冲正过渡科目 |

一笔 CPA 计费的分录：

```
txn_id = uuid()
  D  tenant_receivable   $2.00
  C  platform_revenue    $2.00
```

退款冲正（**不修改原分录**）：

```
txn_id = uuid()
  D  platform_revenue    $2.00
  C  tenant_receivable   $2.00
  ref_type = 'reversal', ref_id = <原 billable_event_id>
```

**不变量（每日校验任务，失败即告警）**：
- 任一 `txn_id` 下 `sum(D) == sum(C)`
- `sum(所有分录 D) == sum(所有分录 C)`
- `billable_event(status = 'billed')` 的金额合计 ==
  `platform_revenue` 科目中 **`ref_type = 'billable_event'`** 部分的净额

> 第三条的 `ref_type` 限定是实现阶段补上的：平台订阅费同样计入
> `platform_revenue`，但它不对应任何 `billable_event`。不加限定的话，每个有
> 订阅费的租户每月都会误报一次「账本不平」—— 一个有噪声的账本告警等于没有
> 告警。落地实现见 `apps/api/src/jobs/audit.rs`。

---

## 4. 归因链路详设

### 4.1 全流程

```
[1] 群内点击
    https://t.me/<bot>?startapp=<tracking_id>
    tracking_id: 10位 base62 随机串（不可枚举）
    → 落 click 事件（ClickHouse），不入 Postgres

[2] Mini App 打开 → 无感登录
    校验 initData: HMAC-SHA256(bot_token) + auth_date ≤ 300s
    多租户：先从 tracking_id 定位 tenant → 取对应 bot_token 校验
    → 签发短期 JWT（15min）+ refresh token
    → upsert Player(tenant_id, tg_user_id)

[3] 游戏交互
    抽奖结果 100% 服务端生成（前端只播动画）
    idempotency_key（客户端 UUID）→ Redis SETNX，TTL 24h
    → 风控 L1 同步检查（见 §7）
    → 落 game_completed 事件

[4] 领奖码签发
    POST /v1/claims  →  ClaimCode {
      code: 8位，字符集排除 0/O/1/I/l
      binds: (player_id, campaign_id, link_id, kol_id)
      status: 'issued'
      expires_at: now + 24h
    }
    Android: store_url + &referrer=<code>   → 首启自动读取
    iOS:     store_url，同时写剪贴板 + UI 明示码
    已安装:  universal link 直接带 code 唤起

[5] 核销（主 App 调用）—— 单事务
    POST /v1/claims/redeem
      { claim_code, app_user_id, device_id }
    ┌─ BEGIN
    │  SELECT ... FOR UPDATE   -- 防并发重复核销
    │  校验 status='issued' && now < expires_at
    │  UPDATE claim_code SET status='redeemed'
    │  UPDATE player SET app_user_id=?, device_ids=append(?)
    │  INSERT attribution (method='deterministic_code', confidence=100,
    │                      is_billable=true, evidence={...})
    │  -- 仅当租户拥有 billing.performance 权益时才建可计费事件（C4）；
    │  -- 否则该租户为纯平台费模式，只写 attribution，不进计费流。
    │  INSERT billable_event (type='activation', status='pending')
    └─ COMMIT
    → 返回 { attributed: true, kol_id, reward_grant_token }

[6] 变现回传
    POST /v1/postback/purchase
      Header: X-Ignition-Key / X-Ignition-Timestamp / X-Ignition-Signature
              签名 = HMAC-SHA256(api_secret, ts + "." + METHOD\n路径\n请求体)
      Body:   { app_user_id, transaction_id, amount, currency, occurred_at }
    → 时间戳窗口 ±5min 防重放
    → transaction_id 幂等
    → 查 attribution → 若 is_billable 则创建 billable_event
      （MVP 阶段 purchase 只入分析流，不计费；CPA 验证后再开启）
```

### 4.2 归因规则（需公开）

这些规则要写成客户可读的文档，并对应 `policy_version`：

| 规则 | v1 取值 | 说明 |
|---|---|---|
| 归因模型 | Last-touch | 最后一次点击的 KOL 获得归因 |
| 点击归因窗口 | 7 天 | 超过则该点击失效 |
| 领奖码有效期 | 24 小时 | 超时需重新完成游戏 |
| 单归因 | 是 | 一个 Player 只归属一个 KOL，不拆分 |
| 归因锁定期 | 90 天 | 期内该 Player 的转化都算原 KOL |
| 可计费方式 | code / referrer / universal_link | 其余仅统计 |
| 冷静期 | activation 7 天 | 期内可冲正 |

**这张表就是产品本身。** 规则变更必须发新版本号 + 提前 30 天通知，老客户可在一个账期内沿用旧版本。

### 4.3 iOS 的现实处理

再强调一次，因为这是最容易被工程实现"想当然"的地方：

- iOS 上**没有**可靠的 user-level deferred deep link。不要在设计中依赖它。
- 剪贴板匹配（`clipboard_match`）**保留但标记为不可计费**——它能提升看板转化率，但不进账单。
- iOS 侧唯一的可计费路径是：用户手动输入领奖码，或已安装用户走 Universal Link。
- **因此，领奖码的输入体验是 iOS 侧收入的直接决定因素。** 主 App 首启弹窗、大输入框、剪贴板检测预填、错误容忍（大小写不敏感、自动去空格）——这些不是打磨，是核心功能。

---

## 5. 计费系统

### 5.1 两条计费轨道

```
轨道 A：订阅（Stripe Subscription）
  Plan → Stripe Price → 月度自动扣款
  Webhook 处理：created / updated / deleted / payment_failed
  失败宽限：7 天，之后降级到只读（不断服，断服会丢客户）

轨道 B：用量（月末结算）
  月末 T+1 跑账单任务：
    1. 拉取该租户上月所有 status='cleared' 的 billable_event
    2. 按 event_type 应用单价
    3. 应用封顶（见 5.2）
    4. 减去上月 reversed 产生的 credit
    5. 生成 Invoice + InvoiceLine
    6. 推送 Stripe Invoice Item → 与订阅合并出账
```

**为什么用量不做实时计费**：CPA 事件有 7 天 hold 期，实时计费必然要频繁冲正，客户体验极差。月末批量结算 + hold 期天然对齐。

### 5.2 封顶（Cap）实现

封顶是给客户的确定性承诺，实现上有个坑：**Redis 计数器在并发和重启下不可靠，但它又是唯一能做实时拦截的地方。**

采用「Redis 预检 + Postgres 权威」双层：

```
写入 billable_event 时：
  1. Redis INCR cap:{tenant}:{yyyymm}  → 得到 n
  2. 若 n > cap_limit，仍然写入事件，但 status 不变
     （事件本身是业务事实，不能因为封顶就不记录）
  3. 事件照常走 hold → cleared

月末结算时（权威）：
  1. 从 Postgres 按 cleared_at 排序取事件
  2. 累加到 cap_limit 为止，之后的标记 over_cap=true
  3. over_cap 事件不计入 invoice，但在看板显示为「已超出封顶（免费）」
```

**关键取舍**：超出封顶的转化**照常归因、照常展示、照常给 KOL 记功，只是不向客户收费**。这比"超出后停止服务"体验好得多，而且是很好的升档话术——客户看到"本月免费送了你 300 个转化"，续费和升档意愿都会上升。

### 5.3 定价配置

```sql
CREATE TABLE pricing_config (
  id              BIGSERIAL PRIMARY KEY,
  tenant_id       BIGINT,               -- NULL = 全局默认
  effective_from  TIMESTAMPTZ NOT NULL,
  effective_to    TIMESTAMPTZ,          -- NULL = 当前生效

  platform_fee_cents  BIGINT NOT NULL,  -- 月度平台费
  cpa_rates       JSONB NOT NULL,       -- { "activation": 200, ... } 单位 cent
  monthly_cap_cents BIGINT,             -- NULL = 无封顶
  currency        CHAR(3) NOT NULL
);
```

**必须有 `effective_from/to`**。种子客户会有优惠价，将来要平滑涨价——涨价是改配置加一条新记录，不是 UPDATE 老记录。已出的账单永远能追溯到当时的价格。这个字段现在加是零成本，事后加是数据迁移。

### 5.4 对账与申诉

三份必备的对外能力（**这是产品功能，不是内部工具**）：

1. **明细导出**：客户可导出任一账期的全部 `billable_event`，含 `external_id`、`occurred_at`、`attribution.method`、`evidence` 摘要。客户能拿去和自己的数据库逐条比对。
2. **差异视图**：并排显示「我方统计」vs「客户回传总数」，自动标出只在一侧存在的记录。**主动暴露差异**是这个产品建立信任的核心动作。
3. **申诉通道**：客户/KOL 对某条记录提异议 → 冻结该条 → 人工复核 → 结论写入 `evidence.appeal`，可冲正。申诉记录本身也要可查询。

---

## 6. 游戏与奖励

相对标准，只列容易出事的点：

- **抽奖服务端权威**：结果由服务端 CSPRNG 生成，前端只播动画。奖池按权重配置，中奖判定后立即扣减库存（乐观锁 + version 字段）。
- **发奖幂等**：`(player_id, campaign_id, idempotency_key)` 唯一索引，重复请求返回首次结果而非报错。
- **奖池库存**：`reward_item.remaining` 用 `UPDATE ... WHERE remaining > 0 RETURNING` 原子扣减，不要先查后改。
- **频次控制**：每 Player 每 Campaign 每日次数，用 Redis 计数 + Postgres 兜底（Redis 挂了要 fail-closed 拒绝抽奖，不是 fail-open 放行）。
- **模板配置用 JSONB + schema 校验**：`template.config_schema` 存 JSON Schema，`campaign.config` 存实例，保存时校验。这样新增模板不需要改表结构，服务于上层的「专属模板」增值。

### 6.1 两种玩法形态

前五个模板（转盘 / 刮刮卡 / 老虎机 / 盲盒 / 翻牌）都是**同一次服务端抽奖的动画皮肤**，
换模板不触碰抽奖、计费、归因任何一行代码。`daily_budget`（每日理财决策）是第二种形态：

- 玩家做一次**有评分的选择**，服务端给分、给即时反馈和科普，跨天累积「理财分」、
  连续打卡与排行榜。选项的分值只存在服务端（`daily_scenario.options`），下发给客户端的
  投影只有 `key + label` —— 与奖池只下发 `id + label`、不下发权重和库存是同一条规则。
- **场景库是平台参考数据**（`daily_scenario`，随 schema 维护），不是租户数据：同一天同一
  campaign 内所有玩家拿到同一道题，排行榜才有可比性；轮换由 `(日期 + campaign_id) % 场景数`
  决定，是纯函数，不是随机。
- **一天一轮由数据库保证**：`daily_round` 上 `(tenant_id, player_id, campaign_id, play_date)`
  唯一索引，重复提交是幂等重放而不是报错。
- **它是留存层，不是计费输入**：答题不发放抽奖次数，`daily_play_limit` 仍是抽奖次数的唯一
  来源；决策 → 抽奖只是界面上的先后顺序。玩家能影响的分数一旦能解锁额外抽奖，就等于让
  互动层去动奖池成本和账单 —— 这正是 C1 禁止的。
- 高分玩家可见的软引导文案来自 `campaign.config.promo`（客户自己的话术），不写进场景库
  （游戏自己的口径保持中立）。

---

## 7. 风控

风控在这个系统里有**双重身份**：既保护奖励成本，也保护计费准确性。后者更重要——一条被判定为作弊的转化，如果已经收了客户的钱，损害的是信任而不只是钱。

### L1 硬约束（同步，在线拦截）

| 规则 | 默认阈值 | 触发动作 |
|---|---|---|
| 单 Player 单 Campaign 日抽奖次数 | 3 | 拒绝 |
| 单 device_id 绑定 Player 数 | 3 | 拒绝核销 |
| 单 IP 日核销数 | 10 | 标记 `held` |
| TG 账号年龄（由 tg_user_id 数值粗判） | 新号 | 标记 `held`，不拒绝 |
| 点击→完成耗时 | < 1.5s | 标记 `held` |

**注意**：L1 对**抽奖**可以直接拒绝，但对**核销**尽量只标记 `held` 不拒绝。误杀一个真实用户的领奖体验，损失大于放过一个刷子——刷子可以事后冲正，被误杀的用户不会回来。

### L2 信号采集（异步，只存不判）

从第一天就存，**这些数据事后无法补采**：

```
IP / ASN / 国家 · device_id · UA · TG账号属性(premium/头像/username/language)
· 行为时序(click→open→complete→redeem 各段耗时)
· 核销时的设备与点击时的地区是否一致
```

### L3 渠道级扫描（异步批处理）

渠道级信号比单用户判定有效得多，且更适合作为暂停结算的依据：

| 信号 | 判定 |
|---|---|
| KOL 完成率 > 全站 P95 | 可疑 |
| KOL 带来用户的 D1 留存 < 全站 P5 | 高度可疑 |
| 核销时间分布的方差异常小 | 脚本特征 |
| 同 ASN 占比 > 60% | 机房流量 |

触发后：该 KOL 本期的 `billable_event` 批量置 `held`，通知客户与 KOL，进人工复核。

**合同层面必须预埋**：「平台有权对疑似异常流量暂停结算并要求补充验证」。技术拦不住所有作弊，能停止付款才是最后的保险。

---

## 8. 多租户、订阅与能力门控

### 隔离

- 所有业务表带 `tenant_id`，且**开启 PostgreSQL RLS**。应用层通过 `SET LOCAL app.tenant_id` 传递，策略强制过滤。这是 C5 的兜底——代码漏写 where 条件时数据库拦住。
- **必须用 `SET LOCAL` 而非 `SET`**：后者作用域是连接，会被连接池里下一个租户的请求复用，直接造成跨租户数据泄漏。
- **配置类表需要单独的策略。** 通用策略写成 `tenant_id = current_setting(...)`，而 `tenant_id IS NULL` 表示「全局默认」（典型是 `pricing_config` 的兜底定价）—— NULL 参与比较恒为 false，这类行会被策略挡掉。后果不是报错而是**查不到定价、金额静默算成 0**，账单全错。这个坑在实现阶段真实踩到过（见 commit `ff44891`）。凡是「租户专属 + 全局默认」两级结构的表，策略都要显式放行 `tenant_id IS NULL`。
- Bot token 用 KMS 信封加密存储，解密后只在内存中存在，**禁止进日志**（日志脱敏中间件里加 token 模式匹配）。
- 跨租户查询只有一个入口：平台管理后台，独立鉴权，全量审计日志。

### 能力门控（C4）

不要写 `if plan == 'pro'`。用 entitlement 驱动：

```sql
CREATE TABLE plan_entitlement (
  plan_id   BIGINT NOT NULL,
  key       TEXT NOT NULL,     -- 'channel.discord' | 'analytics.cohort' | ...
  value     JSONB NOT NULL,    -- true | { "limit": 5 }
  PRIMARY KEY (plan_id, key)
);

CREATE TABLE tenant_entitlement_override (   -- 销售谈判用
  tenant_id BIGINT NOT NULL,
  key       TEXT NOT NULL,
  value     JSONB NOT NULL,
  expires_at TIMESTAMPTZ,
  PRIMARY KEY (tenant_id, key)
);
```

代码里统一：`ent.Check(ctx, "channel.discord")` / `ent.Limit(ctx, "channel.count")`。

**override 表是必须的**：早期销售一定会承诺「给你多开两个群」「先免费用着 Discord」。没有 override 表，这些承诺就会变成硬编码的 if 或者一个假的 plan，三个月后没人说得清某个客户到底买了什么。

初始 entitlement key 规划：

```
channel.count           群组数量上限
channel.discord         Discord 扩展          ← 上层增值
channel.whatsapp        WhatsApp 扩展         ← 上层增值
template.custom         专属模板               ← 上层增值
analytics.basic         三指标看板
analytics.cohort        分层/留存分析          ← 上层增值
branding.whitelabel     白标                  ← 上层增值
marketplace.kol         KOL 撮合              ← 上层增值
billing.performance     是否启用效果分成
export.raw              明细导出
```

### 订阅生命周期

```
trialing ──► active ──► past_due ──► canceled
    │           │  ▲         │
    │           │  └─────────┘  (支付成功恢复)
    │           ▼
    │        paused (客户主动暂停，保留数据)
    └──► canceled (试用未转化)
```

- **试用期 14 天**，不要信用卡（降低冷启动阻力，与商业策略一致）。试用期 entitlement 给到 Pro，让客户看见天花板。
- `past_due` **宽限 7 天且不断服**，只发提醒。断服会导致 KOL 侧链路失效，损害的是客户的客户，客户会直接流失而不是补款。
- 宽限期后降级为**只读**：看板可看、数据可导，但链接停止分发。保留数据 90 天。

---

## 9. 数据与看板

### 分层

```
事件明细 (ClickHouse, 保留 13 个月)
    ↓ 物化视图，按 (tenant, kol, campaign, day) 预聚合
指标层 (ClickHouse MV)
    ↓
看板 API  ──┬── KOL 视角（只看自己的渠道）
            └── 客户视角（全渠道 + 对比）

账单数据 (PostgreSQL, 永久保留)
    ↓
账单 API  ──── 明细导出 / 差异视图 / 申诉
```

### MVP 看板指标（严格控制在这些，不要加）

| 指标 | 来源 | 备注 |
|---|---|---|
| 群内点击量 | ClickHouse | |
| 游戏完成量 | ClickHouse | |
| 领奖码签发量 | ClickHouse | |
| **激活量（已确认）** | PostgreSQL, `cleared` | 计费口径 |
| **激活量（待确认）** | PostgreSQL, `pending`+`held` | 明确标注「hold 期内」 |
| 归因转化（含不可计费） | ClickHouse | 与上面的差值要能解释 |

**最后一行是刻意加的。** 客户会发现"看板说 500，账单只有 420"，与其让他自己发现并质疑，不如系统主动拆解给他看：420 确定性 + 60 概率归因（不计费）+ 20 风控暂缓。

---

## 10. 对外 API 契约要点

主 App 侧的接入成本直接决定 SaaS 的销售阻力。**目标：主 App 只需要接两个接口 + 加一个输入框。**

```
POST /v1/claims/redeem          核销领奖码（必接）
POST /v1/postback/purchase      变现回传（可选，MVP 可后接）

GET  /v1/attribution/:app_user_id   查询归因（可选，用于 App 内展示邀请人）
```

TMA 前端另有一组接口，与 S2S 那套凭据完全分开 —— 前端保不住长期密钥：

```
POST /v1/tma/session            initData → 短期 JWT（access 15min / refresh 7d）
POST /v1/tma/session/refresh
POST /v1/tma/play               抽奖（结果服务端生成，需幂等键）
POST /v1/tma/claim              为一次抽奖签发领奖码

GET  /v1/tma/daily              每日理财决策：今日场景 + 我的分数（daily_budget 模板）
POST /v1/tma/daily/answer       提交今日选择（一天一轮，重复提交幂等重放）
GET  /v1/tma/daily/leaderboard  排行榜
```

`daily/answer` 不需要幂等键：**日期本身就是幂等键**（见 §6.1），这也是它与 `play` 的唯一差别。

**S2S 与回传统一用一套 API Key。** 原设计里回传单独用 `app.postback_secret`，
实现时合并成了 `api_key` + `scopes`：两套密钥意味着两套轮换流程和两个可能过期
的地方，而 scope 已经足以表达「这把钥匙只能核销，不能报账」。相应地
`app.postback_secret_enc` 列在 `db/migrations/0002` 中被删除。

**签名覆盖「方法 + 路径 + 请求体」而不只是请求体。** 只签请求体的话，一个对
`/v1/claims/redeem` 合法的签名可以被原样重放到 `/v1/postback/purchase` 上。

**契约设计原则**：

- 认证用 API Key + HMAC 签名，不用 OAuth（降低接入成本）。
- 所有写接口幂等，重复请求返回首次结果 + `"idempotent": true`。
- 错误码明确区分「可重试」和「不可重试」，并在文档中给出建议重试策略。
- **提供沙箱环境和测试用领奖码**，客户能在不接生产的情况下跑通全流程。这一条对缩短销售周期的作用被严重低估。

---

## 11. 技术选型

| 层 | 选型 | 说明 |
|---|---|---|
| 后端 | Rust + axum + tokio | 见下方「为什么是 Rust」 |
| 数据访问 | sqlx（`derive`，不启用 `macros`） | `query!` 宏要求编译期能连上数据库，会让 CI 和新检出都依赖一个跑着的 Postgres；代价是失去编译期 SQL 校验 |
| 主库 | PostgreSQL 16+ | 账本事务 + RLS + JSONB，三个能力都用得上 |
| 缓存 | Redis 7 | 幂等键 / 限流 / 封顶预检 / 会话 |
| 队列 | Redis Stream → Kafka | MVP 别上 Kafka |
| 分析 | ClickHouse | 物化视图预聚合 |
| 支付 | Stripe | 订阅 + Invoice Item 用量计费，海外标配 |
| TMA | React + Vite + Tailwind + `@telegram-apps/sdk` | 转盘用 CSS transform + cubic-bezier，不上游戏引擎 |
| Console | Next.js + shadcn/ui | 与 TMA 复用 design token |
| 密钥 | KMS / Vault | Bot token、postback secret |
| 可观测 | OpenTelemetry + Grafana | 计费链路要有独立 SLO |

**计费链路的 SLO 要单独定**：核销接口 P99 < 300ms、可用性 99.9%、事件零丢失。这条链路挂了 = 客户的用户领不到奖 = 直接的收入和口碑损失，等级高于游戏链路。

### 为什么是 Rust

这个系统的产出物是账单，而账单错了比服务挂了更伤 —— 挂了客户会打电话来，算错了客户可能几个月后才发现，那时信任已经没了。所以选型的首要标准是**能把多少不变量从「靠约定」搬到「靠编译器」**。

落地后有两条实际做到了：

- **`AttributionMethod::is_billable` 用穷尽 `match` 而非查表。** 新增一种归因方式时不写明它是否可计费就编译不过。用 map 实现的话，忘了登记会静默落到零值 —— 而零值恰好是 `false`，看起来「安全」，实际上把计费口径的正确性寄托在人记得维护一张表上。
- **`ledger::Txn` 字段私有、只能经 `try_new` 构造**，构造时校验借贷平衡、币种一致、金额为正。**不平衡的交易在类型层面无法表示**，调用方没有「忘了先调 validate」这个选项。

另外金额用 `Cents` newtype 而非裸 `i64`：系统里同时有数量、ID、时长一堆 i64，混用是最容易发生且最难发现的一类错误。

代价要如实讲：编译慢、招人面窄、生态不如 Go/TS 成熟。如果团队里没人写过 Rust，这些代价会在前两个月吃掉一部分工期。**这个选型只在「账单正确性 > 交付速度」成立时才划算** —— 对这个产品我认为成立，但它是个判断，不是事实。

---

## 12. MVP 范围与里程碑

### 明确不做

模板商店 · 任务墙 · Discord · WhatsApp · GMV 分成 · 多归因拆分 · KOL 撮合 · 白标 · 自助注册（前 10 个客户手工开通）

### 做

**W1–2 — 骨架与契约**
- 领域模型 + 迁移 + RLS
- **`/v1/claims/redeem` 与 postback 的接口契约定稿并交付给主 App 团队**（这是关键路径，必须最早启动，因为对方的排期不由你控制）
- initData 校验 + JWT
- Event ingest + ClickHouse 落库

**W3–4 — 核心链路**
- 转盘（服务端权威 + 幂等 + 库存）
- 领奖码签发/核销全流程
- Attribution 写入 + policy v1 文档
- BillableEvent + 复式账本 + 不变量校验任务
- L1 风控 + L2 信号采集

**W5–6 — 商业化闭环**
- Stripe 订阅 + entitlement 门控
- 月末账单任务 + 封顶
- 明细导出 + 差异视图
- 三指标看板

**W7–8 — 种子内测**
- 5–10 个 KOL 实跑
- 目标：**验证 iOS 领奖码输入的完成率**（这是整个模型的最大未知数，如果 iOS 完成率低于 40%，中层定价逻辑需要重新设计）
- 对账演练：跑一次完整的月末结算，人工核对每一笔

### 关键验证指标

| 指标 | 目标 | 不达标的含义 |
|---|---|---|
| 群成员 → 游戏完成 | > 25% | 游戏吸引力不足，换模板 |
| 游戏完成 → 领奖码签发 | > 60% | Deep Link 引导话术问题 |
| **iOS 领奖码核销完成率** | **> 40%** | **中层定价模型需重构** |
| Android 自动归因率 | > 85% | Install Referrer 集成有 bug |
| 账单差异率 | < 1% | 归因或幂等有漏洞，**必须归零才能收费** |

---

## 13. 待确认问题与风险

### 阻塞性问题（影响架构，需先答）

1. **主 App 是自有还是外部客户的？**
   - 自有 → W1-2 的接口契约可以并行改造，节省 2 周
   - 外部 → 必须从第一天设计通用接入层 + 沙箱 + 文档，工程量 +50%

2. **主 App 是否已接 AppsFlyer / Adjust / Branch？**
   - 已接 → 强烈建议做 partner 集成，归因模块工作量减少约 60%，且数据公信力更高
   - 未接 → 按本方案自建

3. **变现是 IAP 还是 Web3 链上？**
   - IAP → 本方案适用
   - 链上 → 回传机制、冲正模型完全不同；若涉及真实资金划转，需先做法务/牌照确认（这可能是比技术更长的关键路径）

### 已识别风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| iOS 领奖码完成率过低 | 中层收入模型不成立 | W7 优先验证；备选方案是 iOS 侧改按 CPC/CPE 计费 |
| 主 App 团队排期不配合 | 整体延期 | W1 就交付契约；提供 mock server 让双方并行 |
| 客户少报 postback | 收入漏损 | MVP 阶段 CPA 基于核销（我方可确认），不依赖客户回传 |
| 种子期数据被刷穿 | 基准数据失真 | L1+L2 第一天上线；种子期人工每日看 L3 报表 |
| 归因规则争议 | 信任崩塌 | policy 版本化 + 公开文档 + 差异视图主动暴露 |

---

## 附：一句话总结

这个系统的本质不是「游戏化增长工具」，而是**一台能出具可信账单的归因机器**。转盘是获客话术，归因链路和账本才是产品。所有工程优先级都应该按这个判断来排。
