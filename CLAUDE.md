# LinkSprout 开发约定

先读 `README.md` 的「四条不可违反的约束」。这个仓库里大量看似绕远的写法，
只有对照那四条才讲得通；不了解就改，很容易把收入正确性改坏。

需要更多背景（为什么这样切模块、为什么计费只认确定性归因、里程碑怎么排）时，
读 `docs/system-design.md`。注意它描述的是目标状态，当前实到哪一步以 README 为准；
**如果你的改动让实现偏离了设计文档，回去改文档，别让它烂掉。**

## 改动前必看

- **改归因或计费逻辑** → 先读 `docs/attribution-policy-v1.md`。那份文档对客户
  公开，代码行为必须与它一致。改规则要发新 `policy_version`，不能改 v1 的语义。
- **改 `src/models.rs` 的归因/状态机映射** → 里面的测试是刻意的护栏，
  不要为了让新逻辑通过而改测试预期。
- **改 `redeem.rs`** → 这是全链路唯一的缝合点，整段必须留在单事务内，
  `SELECT ... FOR UPDATE` 不能去掉。

## 硬性规则

1. **金额一律用 `Cents`**，禁止浮点，也不要退回裸 `i64`。
2. **账本只追加**：不写 `UPDATE ledger_entry`，冲正用 `Txn::reverse`。
   数据库层面已回收应用角色的 UPDATE/DELETE 权限，写了也会失败。
3. **状态变更必须走 `billing::transition`**，不要直接给 `ev.status` 赋值。
4. **租户数据必须走 `db::begin_tenant_tx`**，不要拿 `pool` 直接查租户表 ——
   那样绕过 RLS 上下文，查出来是空集（fail-closed），排查起来很费时间。
5. **新增归因方式时认真对待 `is_billable` 的 match 分支** —— 编译器会强制你
   表态，但表态错了没人拦得住。默认应为不可计费。
6. **Bot token 与 postback secret 禁止进日志**，字段名带 `_enc` 后缀的都是。
7. **新增能力开关走 entitlement**，不写 `if plan == "pro"`。

## 测试

- `cargo test` 是纯单元测试，不依赖数据库，必须始终能跑。单测与被测代码同文件
  （`#[cfg(test)] mod tests`），按 Rust 惯例。
- 避免 sleep：时间一律通过参数注入（`now: DateTime<Utc>`），不读真实时钟。
  `telegram::verify` 和 `hmacsig::verify` 都接受 `now` 就是为此。

## 提交

提交信息用英文。
