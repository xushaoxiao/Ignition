# LinkSprout 开发约定

先读 `README.md` 的「四条不可违反的约束」。这个仓库里大量看似绕远的写法，
只有对照那四条才讲得通；不了解就改，很容易把收入正确性改坏。

## 改动前必看

- **改归因或计费逻辑** → 先读 `docs/attribution-policy-v1.md`。那份文档对客户
  公开，代码行为必须与它一致。改规则要发新 `policy_version`，不能改 v1 的语义。
- **改 `internal/models` 的归因/状态机映射** → `models_test.go` 里的测试是刻意
  的护栏，不要为了让新逻辑通过而改测试预期。
- **改 `redeem.go`** → 这是全链路唯一的缝合点，整段必须留在单事务内，
  `SELECT ... FOR UPDATE` 不能去掉。

## 硬性规则

1. **金额一律用 `int64` 存最小货币单位（cent）**，禁止浮点。
2. **账本只追加**：不写 `UPDATE ledger_entry`，冲正用 `ledger.Reverse`。
   数据库层面已回收应用角色的 UPDATE/DELETE 权限，写了也会失败。
3. **状态变更必须走 `billing.Transition`**，不要直接给 `ev.Status` 赋值。
4. **租户数据必须走 `dao.InTenantTx`**，不要用 `db.Pool` 直接查租户表 ——
   那样绕过 RLS 上下文，查出来是空集（fail-closed），排查起来很费时间。
5. **Bot token 与 postback secret 禁止进日志**，字段名带 `_enc` 后缀的都是。
6. **新增能力开关走 entitlement**，不写 `if plan == "pro"`。

## 测试

- `make test` 是纯单元测试，不依赖数据库，必须始终能跑。
- 需要数据库的测试用 `//go:build integration` 标签，走 `make test-integration`。
- 避免 `time.Sleep`：用可注入的时间参数（`Now time.Time`）而非真实时钟。

## 提交

提交信息用英文。
