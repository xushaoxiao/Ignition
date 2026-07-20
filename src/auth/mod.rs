//! 调用方身份。
//!
//! 系统有两类调用方，认证方式不同，因为威胁模型不同：
//!
//! | 调用方 | 方式 | 为什么 |
//! |---|---|---|
//! | 客户主 App 的服务端（S2S） | API Key + HMAC 请求签名 | 服务端能安全保管长期密钥；签名可防篡改与重放 |
//! | TMA 前端（终端用户浏览器） | initData 换发的短期 JWT | 前端保不住长期密钥，只能给短时效、低权限的凭据 |
//!
//! 认证用 API Key 而不是 OAuth，是刻意的：接入成本直接决定销售阻力，
//! 少一轮授权流程就少一周的客户排期（设计文档 §10）。

pub mod apikey;
pub mod jwt;

pub use apikey::Scope;

/// 认证通过后的 S2S 调用方身份。
///
/// 所有 S2S handler 都从这里取 `tenant_id`，不再有任何一处从请求体或自定义头
/// 里读取租户 —— 那正是被本次改动替换掉的占位实现。
///
/// TMA 侧的身份不走这个类型，而是 [`jwt::Claims`]：两条链路的凭据、时效、
/// 权限模型都不同，合并成一个类型只会让「这个请求到底是谁发的」变模糊。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    pub tenant_id: i64,
    pub api_key_id: i64,
}
