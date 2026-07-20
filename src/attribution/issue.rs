//! 领奖码签发。
//!
//! 这是归因链路的第 4 步：用户在 TMA 里抽中了奖，我们发给他一个码，他带着这
//! 个码去主 App 里核销。**码是 iOS 侧唯一可计费的归因载体** —— iOS 上没有可靠
//! 的 user-level deferred deep link，指纹匹配的精度在 iOS 17+ 之后已经崩塌，
//! 所以「用户手动把码输进主 App」是那一侧仅剩的确定性通道。
//!
//! 因此这个接口返回的不只是一个码，还有两个平台各自的落地引导（见 [`Handoff`]）。

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};

use super::claim_code::new_claim_code;
use super::policy::Policy;
use crate::db;

/// 生成不重复的码的重试次数。
///
/// 31 个字符 × 8 位 ≈ 8.5e11 的码空间，撞码概率极低，但唯一索引仍会偶发拒绝。
/// 撞了就换一个再试，3 次仍失败说明不是撞码而是别的问题，应当报错而不是死循环。
const CODE_RETRIES: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum IssueError {
    #[error("attribution: 该次抽奖不存在")]
    PlayNotFound,
    #[error("attribution: 生成领奖码失败")]
    CodeCollision,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// 签发请求。
#[derive(Debug, Clone)]
pub struct IssueRequest {
    pub tenant_id: i64,
    pub player_id: i64,
    pub campaign_id: i64,
    pub link_id: i64,
    pub kol_id: i64,
    /// 对应哪一次抽奖。同一次抽奖只签发一个码。
    pub game_play_id: i64,
    pub now: DateTime<Utc>,
}

/// 两个平台各自的落地引导。
///
/// Android 走 Install Referrer：把码拼进商店链接，主 App 首启即可自动读取，
/// 用户零操作。iOS 拿不到这个能力，只能把码显示出来让用户记住或复制。
#[derive(Debug, Clone, Serialize)]
pub struct Handoff {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ios_url: Option<String>,
    /// iOS 侧必须把码本身显示给用户 —— 前端不要因为「有链接了」就把它藏起来。
    pub show_code_to_user: bool,
}

/// 签发结果。
#[derive(Debug, Clone, Serialize)]
pub struct IssueResult {
    pub claim_code: String,
    pub expires_at: DateTime<Utc>,
    pub handoff: Handoff,
    /// 命中幂等：这次返回的是首次签发的码。
    pub idempotent: bool,
}

/// 签发服务。
pub struct Service {
    pool: PgPool,
    policy: Policy,
}

impl Service {
    pub fn new(pool: PgPool, policy: Policy) -> Self {
        Service { pool, policy }
    }

    /// 为一次抽奖签发领奖码。
    ///
    /// 幂等键是 `game_play_id`：同一次抽奖重复请求返回同一个码。**不能每次
    /// 都发新码** —— 每个码都是一个待核销的归因载体，重复签发意味着一次抽奖
    /// 可以换来多次可计费的核销。
    pub async fn issue(&self, req: &IssueRequest) -> Result<IssueResult, IssueError> {
        let mut tx = db::begin_tenant_tx(&self.pool, req.tenant_id).await?;

        let handoff = self.handoff_for(&mut tx, req.campaign_id).await?;

        // ---- 幂等 ----
        if let Some(row) =
            sqlx::query("SELECT code, expires_at FROM claim_code WHERE game_play_id = $1")
                .bind(req.game_play_id)
                .fetch_optional(&mut *tx)
                .await?
        {
            tx.commit().await?;
            return Ok(IssueResult {
                claim_code: row.try_get("code")?,
                expires_at: row.try_get("expires_at")?,
                handoff: handoff.with_code(row.try_get::<String, _>("code")?.as_str()),
                idempotent: true,
            });
        }

        // 抽奖记录必须属于这个 player 和这个 campaign。不校验的话，
        // 前端可以拿别人的 play_id 给自己换一个码。
        let play_exists = sqlx::query(
            "SELECT 1 AS ok FROM game_play WHERE id = $1 AND player_id = $2 AND campaign_id = $3",
        )
        .bind(req.game_play_id)
        .bind(req.player_id)
        .bind(req.campaign_id)
        .fetch_optional(&mut *tx)
        .await?;
        if play_exists.is_none() {
            return Err(IssueError::PlayNotFound);
        }

        let expires_at = req.now + self.policy.claim_code_ttl;

        for _ in 0..CODE_RETRIES {
            let code = new_claim_code();
            let inserted = sqlx::query(
                r#"
                INSERT INTO claim_code
                  (tenant_id, code, player_id, campaign_id, link_id, kol_id,
                   game_play_id, status, issued_at, expires_at)
                VALUES ($1,$2,$3,$4,$5,$6,$7,'issued',$8,$9)
                ON CONFLICT (tenant_id, code) DO NOTHING
                RETURNING code
                "#,
            )
            .bind(req.tenant_id)
            .bind(&code)
            .bind(req.player_id)
            .bind(req.campaign_id)
            .bind(req.link_id)
            .bind(req.kol_id)
            .bind(req.game_play_id)
            .bind(req.now)
            .bind(expires_at)
            .fetch_optional(&mut *tx)
            .await?;

            if inserted.is_some() {
                tx.commit().await?;
                return Ok(IssueResult {
                    handoff: handoff.with_code(&code),
                    claim_code: code,
                    expires_at,
                    idempotent: false,
                });
            }
        }
        Err(IssueError::CodeCollision)
    }

    async fn handoff_for(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        campaign_id: i64,
    ) -> Result<HandoffTemplate, sqlx::Error> {
        let row = sqlx::query(
            r#"
            SELECT a.store_url_ios, a.store_url_android
              FROM campaign c JOIN app a ON a.id = c.app_id
             WHERE c.id = $1
            "#,
        )
        .bind(campaign_id)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(match row {
            Some(r) => HandoffTemplate {
                ios: r.try_get("store_url_ios")?,
                android: r.try_get("store_url_android")?,
            },
            None => HandoffTemplate::default(),
        })
    }
}

#[derive(Debug, Default, Clone)]
struct HandoffTemplate {
    ios: Option<String>,
    android: Option<String>,
}

impl HandoffTemplate {
    fn with_code(&self, code: &str) -> Handoff {
        Handoff {
            android_url: self.android.as_deref().map(|u| append_referrer(u, code)),
            ios_url: self.ios.clone(),
            // 恒为 true。哪怕 Android 侧能自动读取，把码显示出来也没有坏处；
            // 而 iOS 侧藏起来就是直接的收入损失。
            show_code_to_user: true,
        }
    }
}

/// 把领奖码拼进 Play 商店链接的 `referrer` 参数。
///
/// Play Install Referrer 会把这个值原样交给首启的 App，用户零操作即可归因 ——
/// 这是 Android 侧转化率远高于 iOS 的原因，也是为什么 iOS 的手动输入体验值得
/// 单独投入打磨。
pub fn append_referrer(store_url: &str, code: &str) -> String {
    let sep = if store_url.contains('?') { '&' } else { '?' };
    format!(
        "{store_url}{sep}referrer={}",
        urlencoding::encode(&format!("ignition_code={code}"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referrer_is_appended_with_the_right_separator() {
        assert_eq!(
            append_referrer("https://play.google.com/store/apps/details", "AB3XY9ZK"),
            "https://play.google.com/store/apps/details?referrer=ignition_code%3DAB3XY9ZK"
        );
        assert_eq!(
            append_referrer(
                "https://play.google.com/store/apps/details?id=com.demo.app",
                "AB3XY9ZK"
            ),
            "https://play.google.com/store/apps/details?id=com.demo.app&referrer=ignition_code%3DAB3XY9ZK"
        );
    }

    /// referrer 的值整体要被 URL 编码：`=` 不转义的话，Play 会把它当成
    /// 另一个查询参数的分隔符，App 首启读到的 referrer 就是残缺的。
    #[test]
    fn referrer_value_is_url_encoded() {
        let url = append_referrer("https://example.com/app", "AB3XY9ZK");
        assert!(url.contains("%3D"), "= 未被编码：{url}");
        assert!(!url.contains("referrer=ignition_code=AB3XY9ZK"));
    }
}
