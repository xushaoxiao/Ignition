# LinkSprout

私域游戏化增长与全链路归因平台。

**这个系统的本质不是「游戏化增长工具」，而是一台能出具可信账单的归因机器。**
转盘是获客话术，归因链路和账本才是产品。所有工程优先级都按这个判断来排。

---

## 当前进度

这是 v0 的骨架，实现了收入链路的核心，**尚不可上线**。已实现与未实现见下方清单。

### 已实现

| 模块 | 位置 | 说明 |
|---|---|---|
| 领域模型与迁移 | `migration/0001_init.sql` | 含 RLS 租户隔离、账本表的 UPDATE/DELETE 权限回收 |
| 归因方式与计费判定 | `internal/models` | 只有确定性归因可计费 |
| 可计费事件状态机 | `internal/models` + `internal/service/billing` | pending→cleared→billed→reversed |
| 复式记账 | `internal/service/ledger` | 借贷平衡校验、冲正 |
| 月度封顶 | `internal/service/billing` | 超出部分免费而非拒绝 |
| 领奖码 | `internal/service/attribution/claimcode.go` | CSPRNG、排除易混字符、格式校验 |
| 核销事务 | `internal/service/attribution/redeem.go` | 全链路唯一缝合点，单事务完成 |
| 归因规则版本化 | `internal/service/attribution/policy.go` + `docs/attribution-policy-v1.md` | 可复算、可申诉 |
| TG initData 校验 | `internal/service/telegram` | HMAC + 时效 + 多租户 token |
| 回传签名校验 | `pkg/hmacsig` | HMAC + 时间戳窗口防重放 |
| 风控 L1 + L2 采集 | `internal/service/risk` | 核销侧优先 hold 不 deny |
| HTTP 服务 | `internal/server` | `/v1/claims/redeem` |

### 未实现（按优先级）

1. **`authTenant` 是占位实现** —— 现在从 `X-Tenant-ID` 头读租户，任何人都能伪造
   身份核销任意领奖码。上线前必须换成 API Key + HMAC。见
   `internal/server/handler_redeem.go` 的 `TODO(auth)`。
2. **`postback_secret_enc` / `bot.token_enc` 尚未接 KMS** —— 表结构和字段名已按
   加密存储设计，但还没有加解密实现。
3. `POST /v1/postback/purchase` 回传接口（签名校验已具备，handler 未写）
4. 月末结算任务、Invoice 生成、Stripe 集成
5. 冷静期到期自动 clear 的定时任务
6. 账本不变量的每日校验任务
7. 抽奖 / 奖池扣减 / 领奖码签发接口
8. entitlement 门控的读取与执行
9. TMA 前端、KOL 后台、ClickHouse 分析流
10. L3 渠道级风控扫描

---

## 快速开始

```bash
make up          # 启动 postgres + redis
make migrate     # 建表 + RLS + 应用角色
make seed        # 演示租户/KOL/活动/领奖码
cp configs/config.example.yaml configs/config.yaml
make run         # 启动 API
```

跑一次核销：

```bash
curl -s -X POST localhost:8080/v1/claims/redeem \
  -H 'Content-Type: application/json' \
  -H 'X-Tenant-ID: 1' \
  -d '{"claim_code":"DEMO2345","app_user_id":"app-user-1","device_id":"dev-1"}'
```

```json
{"attributed":true,"kol_id":1,"campaign_id":1,"method":"deterministic_code","policy_version":"v1","held":false}
```

再调一次同样的请求，会得到 `409 code_used` —— 领奖码不可重复核销。

测试：

```bash
make test    # 单元测试，不需要数据库
make lint    # go vet
```

---

## 四条不可违反的约束

代码里的许多设计取舍只有对照这些约束才讲得通。改动前请先读懂它们。

### C1 计费只依赖确定性归因

概率归因的转化可以进看板，绝不能进账单。`AttributionMethod.IsBillable()` 是
这条约束的唯一执行点，`models_test.go` 里有测试守着它。

背景：iOS 17+ 之后 user-level 的 deferred deep link 已无可靠实现。按概率匹配
计费，等于向客户收一笔我们自己也无法验证的钱。

### C2 归因数据不能有利益冲突

平台费的存在意义就是让我们不靠数字大小赚钱。配套要求：归因规则版本化
（`policy.go`）、对客户公开（`docs/attribution-policy-v1.md`）、每条记录存下
判定时的 `policy_version` 和 `evidence` 快照。

`evidence` 只增不改 —— 它是 KOL 申诉时唯一的证据来源。

### C3 账本只可追加

退款、拒付、事后判定作弊，一律写反向分录，不 UPDATE 原记录。数据库层面已对
应用角色回收 `ledger_entry` 的 UPDATE/DELETE 权限。

`attribution.is_billable` 是冗余字段（逻辑上可由 `method` 推导），但必须存 ——
计费规则会变，而已开出账单的判定依据必须冻结。

### C4 能力门控数据驱动

不允许 `if plan == "pro"` 散落在代码里。用 `plan_entitlement` +
`tenant_entitlement_override` 驱动。

override 表不是可选项：早期销售一定会承诺「先免费给你开 Discord」，没有这张表
承诺就会变成硬编码，三个月后没人说得清客户到底买了什么。

---

## 关键设计说明

### 核销是全链路唯一的缝合点

`internal/service/attribution/redeem.go` 是整个系统最关键的一段代码。TG 侧身份
（`tg_user_id`，来自 initData）与 App 侧身份（`app_user_id`，来自主 App）在这
一刻绑定，归因与可计费事件同时产生。

必须在单事务内完成：如果绑定成功但归因写入失败，这个用户就永远无法被正确归因
了 —— 领奖码已作废，没有第二次机会。

事务里的 `SELECT ... FOR UPDATE` 不是可选的。用户狂点或客户端重试会造成并发
提交，没有这把锁，两个事务都会读到 `issued` 并各写一条归因和一笔计费。

### 为什么 CPA 基于核销而非客户回传

MVP 的计费事件 `external_id` 用 `claim:<id>` —— 核销是我方可确认的事实。

如果按客户回传的 IAP 计费，客户漏传一笔就少付我们一笔，且几乎无法被发现。
纯 take-rate 模式在 IAP 场景下有这个结构性弱点，所以 MVP 先从我方可确认的
事件收费。

### 风控在核销侧优先 hold 不 deny

误杀一个真实用户 = 他领不到奖 + 对客户 App 的第一印象是「这活动是骗人的」，
不可挽回。放过一个刷子只是暂时多算一笔，冷静期内可冲正、可人工驳回，钱还没
真正付出去。

唯一直接拒绝的是设备维度 —— 真实用户几乎不可能一台设备绑三个账号。

### 超封顶不停服

超出月度封顶的转化照常归因、照常给 KOL 记功，只是不计费，看板标注
「已超出封顶（免费）」。比「超出后停服」体验好得多，且是最自然的升档话术。

### 领奖码字符集排除 0/O/1/I/L

手动输入是 iOS 侧唯一可计费的归因路径，一个字符的误读就是一次收入损失。

`NormalizeClaimCode` 刻意**不做**混淆字符映射：把 `0` 猜成 `O` 还是 `D` 无法
可靠推断，猜错会把一个有效码变成另一个有效码，归到错误的 KOL 名下。宁可报
格式错让用户重输。

---

## 目录结构

```
cmd/api/              服务入口
configs/              配置（config.yaml 不进版本库）
migration/            建表迁移 + 演示数据
docs/                 归因规则等对外文档
internal/
  conf/               配置加载
  models/             领域类型、归因计费映射、状态机
  dao/                pgx 连接池、租户事务（RLS 上下文）
  server/             HTTP 接口
  service/
    attribution/      归因规则、领奖码、核销事务
    billing/          状态推进、封顶
    ledger/           复式记账
    risk/             L1 硬约束
    telegram/         initData 校验
pkg/hmacsig/          回传签名
```

## 技术选型说明

标准库路由 + pgx + testify，刻意不引入 Kratos/etcd/wire 那套微服务框架。MVP
阶段最大的风险是交付速度，不是架构优雅度。模块边界已按未来可拆分的方式划清
（`internal/service/*` 之间只通过导出接口互调）。

Redis 已在 `docker-compose.yml` 中就位，但代码尚未使用 —— 幂等键与限流目前
依赖数据库唯一约束，够 MVP 用。
