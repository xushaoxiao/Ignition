# Ignition

私域游戏化增长与全链路归因平台。

**这个系统的本质不是「游戏化增长工具」，而是一台能出具可信账单的归因机器。**
转盘是获客话术，归因链路和账本才是产品。所有工程优先级都按这个判断来排。

## 文档

| 文档 | 内容 | 读者 |
|---|---|---|
| [docs/system-design.md](docs/system-design.md) | 完整系统设计：商业模型到系统能力的映射、领域模型、归因链路、计费与账本、风控、里程碑 | 内部 |
| [docs/attribution-policy-v1.md](docs/attribution-policy-v1.md) | 归因规则 v1 | **对客户与 KOL 公开** |
| [CLAUDE.md](CLAUDE.md) | 改动前必看的硬性规则 | 内部 |

系统设计文档描述的是**目标状态**；下面的「当前进度」才是实际状态。两者冲突时
以本文件为准，并回去修正设计文档。

---

## 当前进度

收入链路已经完整跑通：**TMA 开场 → 抽奖 → 领奖码 → 核销归因 → 计费事件 →
月末出账 → 复式分录 → 账本校验**，端到端可运行。仍有若干上线前必须补齐的项，
见下方「未实现」。

### 已实现

| 模块 | 位置 | 说明 |
|---|---|---|
| 领域模型与迁移 | `migration/0001_init.sql` | 含 RLS 租户隔离、账本表的 UPDATE/DELETE 权限回收 |
| 归因方式与计费判定 | `src/models.rs` | 只有确定性归因可计费，穷尽 match 强制表态 |
| 可计费事件状态机 | `src/models.rs` + `src/billing.rs` | pending→cleared→billed→reversed |
| 复式记账 | `src/ledger.rs` | 不平衡的 `Txn` 在类型层面不可构造 |
| 月度封顶 | `src/billing.rs` | 超出部分免费而非拒绝 |
| 领奖码 | `src/attribution/claim_code.rs` | 排除易混字符、归一化、格式校验 |
| 核销事务 | `src/attribution/redeem.rs` | 全链路唯一缝合点，单事务完成 |
| 归因规则版本化 | `src/attribution/policy.rs` + `docs/attribution-policy-v1.md` | 可复算、可申诉 |
| TG initData 校验 | `src/telegram.rs` | HMAC + 时效 + 多租户 token |
| 回传签名校验 | `src/hmacsig.rs` | HMAC + 时间戳窗口防重放 |
| 风控 L1 + L2 采集 | `src/risk.rs` | 核销侧优先 hold 不 deny |
| HTTP 服务 | `src/server/` | S2S 两个接口 + TMA 四个接口 |
| **API Key + HMAC 认证** | `src/auth/apikey.rs` | 签名覆盖方法 + 路径 + 请求体，scope 缺省拒绝 |
| **密钥加密存储** | `src/secrets.rs` | AES-256-GCM，`Secret` 的 Debug 恒为 `<redacted>` |
| **TMA 会话** | `src/auth/jwt.rs` | initData → access 15min + refresh 7d |
| **服务端权威抽奖** | `src/game/` | 权重抽取 + 原子扣库存 + 幂等 |
| **领奖码签发** | `src/attribution/issue.rs` | 一次抽奖一个码，含双平台落地引导 |
| **变现回传** | `src/attribution/postback.rs` | 分析流与计费流分离 |
| **能力门控** | `src/entitlement.rs` | plan 缺省 + 租户 override，缺省关闭 |
| **定时任务** | `src/jobs/` | 冷静期放行 / 账本校验 / 月末结算 |
| **TMA 前端** | `web/tma/` | React + Vite + Tailwind，转盘用 CSS transform |

### 未实现（按优先级）

1. **主密钥来自环境变量，尚未接 KMS** —— `src/secrets.rs` 的密文格式已经带了
   版本字节，接 KMS 时是加一个 `V2` 分支，旧密文继续可读，不需要停机迁移。
2. **Stripe 集成** —— 月末账单已能生成 `invoice` + `invoice_line` + 复式分录，
   但没有推给支付侧，`invoice.status` 恒为 `draft`。
3. **entitlement 尚无门控点** —— 能力集与订阅服务等级都已实现并有测试，
   但除了「欠费超宽限期停止分发新会话」外，还没有具体能力挂上去。
4. 明细导出 / 差异视图 / 申诉通道（设计文档 §5.4，是产品功能而非内部工具）
5. 冲正链路：`ledger::Txn::reverse` 已实现且有测试，但没有触发它的接口
6. KOL 后台、三指标看板、ClickHouse 分析流
7. L3 渠道级风控扫描
8. 归因查询接口 `GET /v1/attribution/:app_user_id`

---

## 快速开始

### 后端

```bash
cp configs/config.example.yaml configs/config.yaml

# 两把密钥只从环境变量读，缺失即启动失败 —— 不提供默认值，
# 一个有默认值的签名密钥等于所有部署共用同一把钥匙。
export IGNITION_MASTER_KEY=$(cargo run -q -- keygen)
export IGNITION_JWT_KEY=$(openssl rand -base64 32)

make reset       # 起库 + 迁移 + 演示数据 + 演示密钥
make run
```

`make reset` 里的 `secrets` 这一步用 `ignition seal` 现场加密演示用的 Bot token
与 API Key —— 密文不进版本库，否则加密存储就只是个摆设。

### 共享 Supabase

和 growing-tales 共用一个 Supabase 实例时，靠**独立 schema** 隔离（默认
`ignition`，配置项 `postgres.schema` / 环境变量 `IGNITION_PG_SCHEMA`）。
每条连接会把它放到 `search_path` 首位，与 growing-tales 同一套约定。

```bash
export IGNITION_PG_DSN='postgresql://...pooler.supabase.com:5432/postgres?sslmode=require'
make migrate-remote
```

连接池要选 **Session Pooler（5432）而不是 Transaction Pooler（6543）**：
后者不保留会话状态，sqlx 的预编译语句和 `search_path` 都会失效。

**连接角色必须是 `ignition_app`，不能是 `postgres`。** 迁移会建好这个角色
（NOLOGIN 的权限容器），部署时单独开登录：

```sql
ALTER ROLE ignition_app LOGIN PASSWORD '<随机口令>';
```

DSN 里的用户名用 Supavisor 的格式 `ignition_app.<project-ref>`，已验证可用。
口令建议只用字母数字 —— 它要进连接串，特殊字符得 URL 转义，是个常见的坑。

切换后启动日志会打 `已连接数据库，RLS 生效`；打的是 ERROR 说明还连着特权角色。

> **⚠️ 为什么不能用 `postgres` 角色：租户隔离（约束 C5）会不生效。**
>
> 那个角色带 `rolbypassrls`，RLS 策略对它整体不适用 —— 连
> `FORCE ROW LEVEL SECURITY` 都拦不住，那只能约束表 owner，管不了 BYPASSRLS。
>
> 危险之处在于**它没有任何症状**：接口照常返回、测试照常通过，只有当某个
> 客户看到别人的数据时才会暴露。所以服务启动时会查一次 `pg_roles` 并打
> ERROR 级日志，但那只是提醒，不是补救。
>
> `ignition_app` 之所以被建成 NOLOGIN、要单独开登录：迁移会在公网可达的
> 托管库上执行，带默认口令的登录角色等于一个人人都猜得到的入口。

账本的只可追加（约束 C3）在托管库上也换了实现：原来靠
`REVOKE UPDATE, DELETE`，但权限回收对表 owner 无效。现在额外加了一个
`BEFORE UPDATE OR DELETE` 触发器，对所有角色生效 —— 包括 owner 和 BYPASSRLS。

### 定时任务

由外部调度器拉起，刻意不塞进服务进程：账单任务需要能被人手重跑、能单独看日志。

```bash
make job-clear     # 每小时：冷静期到期的事件转入 cleared
make job-audit     # 每日：校验账本不变量，失败即非零退出（接告警）
make job-settle    # 每月 T+1：出上个月的账单
```

### TMA 前端

```bash
cd web/tma
pnpm install
cp .env.example .env.local     # 填 VITE_API_BASE
pnpm dev
```

Telegram 只加载 HTTPS 页面，本地调试需要 cloudflared / ngrok 之类的隧道。
不想起隧道时，可以注入一份预先签好的 initData 在浏览器里跑通全流程，
见 [web/tma/README.md](web/tma/README.md)。

### 测试

```bash
make test    # cargo test，不需要数据库
make lint    # cargo clippy + fmt --check
```

---

## 四条不可违反的约束

代码里的许多设计取舍只有对照这些约束才讲得通。改动前请先读懂它们。

### C1 计费只依赖确定性归因

概率归因的转化可以进看板，绝不能进账单。`AttributionMethod::is_billable()` 是这条约束的唯一执行点，
它用穷尽 `match` 而非查表实现：新增一种归因方式时编译器会强制你明确表态，
不可能因为忘了登记而默认落到某个分支。`models.rs` 的测试另有一道守护。

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

`src/attribution/redeem.rs` 是整个系统最关键的一段代码。TG 侧身份
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

`normalize_claim_code` 刻意**不做**混淆字符映射：把 `0` 猜成 `O` 还是 `D` 无法
可靠推断，猜错会把一个有效码变成另一个有效码，归到错误的 KOL 名下。宁可报
格式错让用户重输。

---

## 目录结构

```
configs/              配置（config.yaml 不进版本库）
migration/            建表迁移 + 演示数据
docs/                 归因规则等对外文档
src/
  main.rs             入口：服务 / 定时任务 / 密钥工具
  config.rs           配置加载（密钥只从环境变量读）
  models.rs           领域类型、归因计费映射、状态机、Cents
  db.rs               连接池、租户事务（RLS 上下文）
  secrets.rs          _enc 字段的加解密与防泄漏封装
  ledger.rs           复式记账
  billing.rs          状态推进、封顶
  entitlement.rs      能力门控、订阅服务等级
  risk.rs             L1 硬约束
  telegram.rs         initData 校验
  hmacsig.rs          HMAC 签名与时间窗
  auth/               API Key 签名（S2S）、会话令牌（TMA）
  attribution/        归因规则、领奖码签发与核销、变现回传
  game/               服务端权威抽奖与奖池扣减
  jobs/               冷静期放行、账本校验、月末结算
  server/             HTTP 接口
web/tma/              Telegram Mini App 前端
```

## 技术选型说明

Rust + axum + sqlx + tokio。

**sqlx 只启用 `derive` 而非 `macros`**：`query!` 系列宏要求编译期能连上数据库，
会让 CI 和新同事的第一次 `cargo build` 都依赖一个跑着的 Postgres。代价是失去
编译期 SQL 校验，换来构建无外部依赖。

**类型系统承担了两条核心不变量**，这是相对动态检查的实质收益：

- `AttributionMethod::is_billable()` 用穷尽 `match` —— 新增归因方式时编译器
  强制你决定它是否可计费，忘记登记不会静默落到默认分支。
- `ledger::Txn` 字段私有、只能经 `try_new` 构造 —— **不平衡的交易在类型层面
  无法存在**，不依赖调用方记得先调一次校验。

`Cents` 是 newtype 而非裸 `i64`：系统里同时有数量、ID、时长等一堆 i64，
混用是最容易发生且最难发现的一类错误。

Redis 已在 `docker-compose.yml` 中就位，但代码尚未使用 —— 幂等键与限流目前
依赖数据库唯一约束，够 MVP 用。

`main.rs` 顶部的 `#![allow(dead_code)]` 是有期限的：账本、封顶、回传签名都已
实现并有测试，但还没接入在线链路。那两块接完后应移除该 allow，让 `dead_code`
重新变成有效信号。
