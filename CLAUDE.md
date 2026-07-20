# Ignition 开发约定

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
- **改 S2S 接口的入参解析** → handler 必须先按**原始 `Bytes`** 验签再反序列化。
  改成 `Json<T>` 提取器会让签名对不上，且症状是「部分客户随机 401」。
- **改 `jobs/settle.rs`** → 它是账本、封顶、状态机唯一同时进入在线链路的地方。
  取待结算事件的条件是 `invoice_id IS NULL` 而**不是**按账期窗口过滤 ——
  换成窗口过滤，晚放行的事件会被永远漏掉。

## 硬性规则

1. **金额一律用 `Cents`**，禁止浮点，也不要退回裸 `i64`。
2. **账本只追加**：不写 `UPDATE ledger_entry`，冲正用 `Txn::reverse`。
   数据库层面已回收应用角色的 UPDATE/DELETE 权限，写了也会失败。
3. **状态变更必须走 `billing::transition`**，不要直接给 `ev.status` 赋值。
4. **租户数据必须走 `db::begin_tenant_tx`**，不要拿 `pool` 直接查租户表 ——
   那样绕过 RLS 上下文，查出来是空集（fail-closed），排查起来很费时间。
5. **新增归因方式时认真对待 `is_billable` 的 match 分支** —— 编译器会强制你
   表态，但表态错了没人拦得住。默认应为不可计费。
6. **密钥禁止进日志**。`_enc` 后缀的字段解密后一律包在 `secrets::Secret` 里，
   它的 `Debug` 恒为 `<redacted>`；要拿明文必须显式调 `expose()`，
   那是一处能被 review 抓住的调用。不要为了打日志方便把它拆成 `String`。
7. **新增能力开关走 entitlement**，不写 `if plan == "pro"`。
8. **租户身份只能来自 `auth::Caller` 或 `jwt::Claims`**，不许从请求体、
   查询参数或自定义头里读 —— 那正是被 API Key 签名替换掉的占位实现。
9. **Postgres 的 `sum(bigint)` 返回 `NUMERIC`**，聚合金额时一律显式 `::bigint`，
   否则 sqlx 会在解码时报类型不匹配。这个坑在结算与账本校验里都踩到过。

## 前端

TMA 前端在 `web/tma/`，约定见 [web/tma/README.md](./web/tma/README.md)。两条不能改：
抽奖结果由服务端产生（前端只播动画），initData 必须原样上传（签名对原始字段序列算）。

## 测试

- `cargo test` 是纯单元测试，不依赖数据库，必须始终能跑。单测与被测代码同文件
  （`#[cfg(test)] mod tests`），按 Rust 惯例。
- 避免 sleep：时间一律通过参数注入（`now: DateTime<Utc>`），不读真实时钟。
  `telegram::verify` 和 `hmacsig::verify` 都接受 `now` 就是为此。

## 提交

提交信息用英文。
