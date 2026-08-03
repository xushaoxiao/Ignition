//! One daily decision round.
//!
//! The whole surface is three reads and one write: today's scenario, today's answer, the
//! leaderboard. Everything the player can influence stays inside `daily_round`; nothing here
//! touches `billable_event`, `ledger_entry`, or the claim code — the decision game drives return
//! visits, the redeem path drives invoices, and they must not meet (constraint C1).

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};

use super::{Choice, Grade, Promo, PublicScenario, Scenario};
use crate::db;

/// Leaderboard page size. Deliberately short: the point is "am I near the top", and a long list on
/// a phone is scrolled past, not read.
const BOARD_LIMIT: i64 = 20;

#[derive(Debug, thiserror::Error)]
pub enum DailyError {
    #[error("daily: campaign not found or not active")]
    CampaignInactive,
    #[error("daily: scenario catalog is empty")]
    NoScenario,
    #[error("daily: unknown choice key")]
    UnknownChoice,
    #[error("daily: scenario catalog is malformed")]
    Catalog(#[from] serde_json::Error),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Identity of the player asking, always taken from the session token — never the request body.
#[derive(Debug, Clone)]
pub struct Player {
    pub tenant_id: i64,
    pub player_id: i64,
    pub campaign_id: i64,
    /// UTC date of the round. Injected rather than read from the clock (or from `now()` in SQL) so
    /// streak behaviour around midnight and month boundaries is testable and does not depend on
    /// the database server's timezone.
    pub date: NaiveDate,
}

impl Player {
    pub fn from_now(tenant_id: i64, player_id: i64, campaign_id: i64, now: DateTime<Utc>) -> Self {
        Player {
            tenant_id,
            player_id,
            campaign_id,
            date: now.date_naive(),
        }
    }
}

/// What one answered round produced. Also the shape replayed on a duplicate submit.
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub choice_key: String,
    pub choice_label: String,
    /// The decision's own score change, kept apart from the streak bonus so the result screen can
    /// credit the habit separately from the answer.
    pub delta: i32,
    pub streak_bonus: i32,
    pub credit: i32,
    pub grade: Grade,
    pub grade_label: &'static str,
    pub streak: i32,
    pub verdict: String,
    pub tip: String,
}

/// Today's screen: the scenario, where the player stands, and their answer if they already gave one.
#[derive(Debug, Clone, Serialize)]
pub struct TodayView {
    pub date: NaiveDate,
    pub scenario: PublicScenario,
    pub credit: i32,
    pub grade: Grade,
    pub grade_label: &'static str,
    pub streak: i32,
    pub rounds_played: i64,
    /// `Some` once today's decision is made. The frontend renders the result screen straight away
    /// instead of offering a second answer that the database would reject.
    pub answered: Option<Outcome>,
    pub rank: Option<i64>,
    pub players: i64,
    pub promo: Option<Promo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnswerResult {
    #[serde(flatten)]
    pub outcome: Outcome,
    pub rank: i64,
    pub players: i64,
    pub promo: Option<Promo>,
    /// This response repeats an answer already recorded for today.
    pub idempotent: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BoardEntry {
    pub rank: i64,
    pub name: String,
    pub credit: i32,
    pub streak: i32,
    pub me: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Leaderboard {
    pub entries: Vec<BoardEntry>,
    pub my_rank: Option<i64>,
    pub players: i64,
}

pub struct Service {
    pool: PgPool,
}

impl Service {
    pub fn new(pool: PgPool) -> Self {
        Service { pool }
    }

    /// Today's scenario plus the player's standing.
    pub async fn today(&self, req: &Player) -> Result<TodayView, DailyError> {
        let mut tx = db::begin_tenant_tx(&self.pool, req.tenant_id).await?;

        let promo = load_campaign(&mut tx, req).await?;
        let catalog = load_catalog(&mut tx).await?;
        let scenario = pick_scenario(&catalog, req)?;

        let today_row = load_round(&mut tx, req, req.date).await?;
        let last = load_last_round(&mut tx, req).await?;
        let rounds_played = count_rounds(&mut tx, req).await?;

        // Answered today → show that round's frozen numbers; otherwise the last round's carry-over,
        // or the starting score for a first-time player.
        let (credit, streak) = match (&today_row, &last) {
            (Some(r), _) => (r.credit_after, r.streak_after),
            (None, Some(r)) => (r.credit_after, r.streak_after),
            (None, None) => (super::CREDIT_START, 0),
        };

        let answered = match &today_row {
            // Look the scenario up by the **stored** id, not by today's rotation: if the catalog
            // grows between the answer and the reload, the rotation may point elsewhere and the
            // player would see feedback for a question they never answered.
            Some(r) => Some(outcome_of(r, find_scenario(&catalog, r.scenario_id))?),
            None => None,
        };

        let standing = standing(&mut tx, req, credit, streak).await?;
        tx.commit().await?;

        Ok(TodayView {
            date: req.date,
            scenario: PublicScenario::from(scenario),
            credit,
            grade: Grade::of(credit),
            grade_label: Grade::of(credit).label(),
            streak,
            rounds_played,
            answered,
            rank: today_row.is_some().then_some(standing.rank),
            players: standing.players,
            promo: promo.filter(|p| p.visible_at(credit)),
        })
    }

    /// Record today's decision.
    ///
    /// A duplicate submit is an idempotent replay, not an error: retries after a flaky connection
    /// are normal, and "你今天已经答过了" as a failure would be indistinguishable from a bug for the
    /// player who never saw the first response. The database decides who was first — the unique
    /// index on `(tenant_id, player_id, campaign_id, play_date)` — so two racing taps cannot both
    /// insert. No `SELECT ... FOR UPDATE` here: unlike `redeem`, nothing downstream is spent, and
    /// the conflicting insert is the lock.
    pub async fn answer(&self, req: &Player, choice_key: &str) -> Result<AnswerResult, DailyError> {
        let mut tx = db::begin_tenant_tx(&self.pool, req.tenant_id).await?;

        let promo = load_campaign(&mut tx, req).await?;
        let catalog = load_catalog(&mut tx).await?;
        let scenario = pick_scenario(&catalog, req)?;
        let choice = scenario
            .choice(choice_key)
            .ok_or(DailyError::UnknownChoice)?;

        let last = load_last_round(&mut tx, req).await?;
        if let Some(prev) = last.as_ref().filter(|r| r.play_date == req.date) {
            let outcome = outcome_of(prev, find_scenario(&catalog, prev.scenario_id))?;
            let standing = standing(&mut tx, req, outcome.credit, outcome.streak).await?;
            tx.commit().await?;
            return Ok(AnswerResult {
                rank: standing.rank,
                players: standing.players,
                promo: promo.filter(|p| p.visible_at(outcome.credit)),
                outcome,
                idempotent: true,
            });
        }

        let (prev_credit, prev) = match &last {
            Some(r) => (r.credit_after, Some((r.play_date, r.streak_after))),
            None => (super::CREDIT_START, None),
        };
        let streak = super::advance_streak(prev, req.date);
        let bonus = super::streak_bonus(streak);
        let credit = super::apply_delta(prev_credit, choice.delta + bonus);

        let inserted = sqlx::query(
            r#"
            INSERT INTO daily_round
              (tenant_id, player_id, campaign_id, play_date, scenario_id, choice_key,
               score_delta, streak_bonus, credit_after, streak_after)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
            ON CONFLICT (tenant_id, player_id, campaign_id, play_date) DO NOTHING
            RETURNING id
            "#,
        )
        .bind(req.tenant_id)
        .bind(req.player_id)
        .bind(req.campaign_id)
        .bind(req.date)
        .bind(scenario.id)
        .bind(&choice.key)
        .bind(choice.delta)
        .bind(bonus)
        .bind(credit)
        .bind(streak)
        .fetch_optional(&mut *tx)
        .await?;

        // Lost the race with a concurrent tap: the other transaction's row is the answer of record.
        // Re-read it rather than reporting a conflict — both taps were the same user intent.
        let (outcome, idempotent) = match inserted {
            Some(_) => (
                Outcome {
                    choice_key: choice.key.clone(),
                    choice_label: choice.label.clone(),
                    delta: choice.delta,
                    streak_bonus: bonus,
                    credit,
                    grade: Grade::of(credit),
                    grade_label: Grade::of(credit).label(),
                    streak,
                    verdict: choice.verdict.clone(),
                    tip: choice.tip.clone(),
                },
                false,
            ),
            None => {
                let row = load_round(&mut tx, req, req.date)
                    .await?
                    .ok_or(sqlx::Error::RowNotFound)?;
                (
                    outcome_of(&row, find_scenario(&catalog, row.scenario_id))?,
                    true,
                )
            }
        };

        let standing = standing(&mut tx, req, outcome.credit, outcome.streak).await?;
        tx.commit().await?;

        Ok(AnswerResult {
            rank: standing.rank,
            players: standing.players,
            promo: promo.filter(|p| p.visible_at(outcome.credit)),
            outcome,
            idempotent,
        })
    }

    /// Campaign leaderboard: the latest round of every player who has ever answered.
    pub async fn leaderboard(&self, req: &Player) -> Result<Leaderboard, DailyError> {
        let mut tx = db::begin_tenant_tx(&self.pool, req.tenant_id).await?;

        // DISTINCT ON keeps one row per player — their most recent round, which is where the
        // current credit score and streak live. Ranking over every round would let a player who
        // played a lot outrank one who played well.
        let rows = sqlx::query(
            r#"
            WITH latest AS (
              SELECT DISTINCT ON (dr.player_id)
                     dr.player_id, dr.credit_after, dr.streak_after
                FROM daily_round dr
               WHERE dr.campaign_id = $1
               ORDER BY dr.player_id, dr.play_date DESC, dr.id DESC
            )
            SELECT l.player_id, l.credit_after, l.streak_after, p.tg_username
              FROM latest l
              JOIN player p ON p.id = l.player_id
             ORDER BY l.credit_after DESC, l.streak_after DESC, l.player_id
             LIMIT $2
            "#,
        )
        .bind(req.campaign_id)
        .bind(BOARD_LIMIT)
        .fetch_all(&mut *tx)
        .await?;

        let mut entries: Vec<BoardEntry> = Vec::with_capacity(rows.len());
        for r in &rows {
            let player_id: i64 = r.try_get("player_id")?;
            let credit: i32 = r.try_get("credit_after")?;
            let streak: i32 = r.try_get("streak_after")?;
            // Competition ranking: ties share a rank, so the number here matches the rank the tied
            // player is shown on their own result screen.
            let rank = 1 + entries
                .iter()
                .filter(|e| (e.credit, e.streak) > (credit, streak))
                .count() as i64;
            entries.push(BoardEntry {
                rank,
                name: display_name(r.try_get("tg_username")?, player_id),
                credit,
                streak,
                me: player_id == req.player_id,
            });
        }

        let mine = load_last_round(&mut tx, req).await?;
        let my_rank = match &mine {
            Some(r) => Some(
                standing(&mut tx, req, r.credit_after, r.streak_after)
                    .await?
                    .rank,
            ),
            None => None,
        };
        let players = count_players(&mut tx, req).await?;
        tx.commit().await?;

        Ok(Leaderboard {
            entries,
            my_rank,
            players,
        })
    }
}

// ---------------------------------------------------------------- storage rows

/// A stored round, read back for replays and for carry-over of credit and streak.
struct Round {
    play_date: NaiveDate,
    scenario_id: i64,
    choice_key: String,
    score_delta: i32,
    streak_bonus: i32,
    credit_after: i32,
    streak_after: i32,
}

struct Standing {
    rank: i64,
    players: i64,
}

/// Verify the campaign is live and read its soft-prompt configuration.
async fn load_campaign(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    req: &Player,
) -> Result<Option<Promo>, DailyError> {
    let row = sqlx::query(
        r#"
        SELECT config
          FROM campaign
         WHERE id = $1 AND status = 'active'
           AND (starts_at IS NULL OR starts_at <= $2)
           AND (ends_at   IS NULL OR ends_at   >  $2)
        "#,
    )
    .bind(req.campaign_id)
    // Midnight UTC of the round's date: the window check only needs day resolution here, and
    // reusing `req.date` keeps the handler free of a second clock reading.
    .bind(req.date.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(DailyError::CampaignInactive)?;

    let config: serde_json::Value = row.try_get("config")?;
    let Some(raw) = config.get("promo") else {
        return Ok(None);
    };
    // A malformed promo is a content mistake, not an outage: log it and run the game without the
    // prompt rather than failing the round the player is in the middle of.
    match serde_json::from_value::<Promo>(raw.clone()) {
        Ok(p) => Ok(Some(p)),
        Err(e) => {
            tracing::warn!(campaign_id = req.campaign_id, error = %e, "campaign.config.promo is malformed, ignored");
            Ok(None)
        }
    }
}

/// The scenario catalog in `id` order — the order [`super::scenario_index`] indexes into.
async fn load_catalog(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Vec<Scenario>, DailyError> {
    let rows =
        sqlx::query("SELECT id, code, title, prompt, options FROM daily_scenario ORDER BY id")
            .fetch_all(&mut **tx)
            .await?;

    rows.into_iter()
        .map(|r| {
            let options: serde_json::Value = r.try_get("options")?;
            Ok(Scenario {
                id: r.try_get("id")?,
                code: r.try_get("code")?,
                title: r.try_get("title")?,
                prompt: r.try_get("prompt")?,
                choices: serde_json::from_value::<Vec<Choice>>(options)?,
            })
        })
        .collect()
}

fn pick_scenario<'a>(catalog: &'a [Scenario], req: &Player) -> Result<&'a Scenario, DailyError> {
    let idx = super::scenario_index(catalog.len(), req.campaign_id, req.date)
        .ok_or(DailyError::NoScenario)?;
    catalog.get(idx).ok_or(DailyError::NoScenario)
}

fn find_scenario(catalog: &[Scenario], scenario_id: i64) -> Option<&Scenario> {
    catalog.iter().find(|s| s.id == scenario_id)
}

/// Rebuild the result screen from a stored round.
///
/// Scores come from the row, not from the current catalog: rebalancing an option's `delta` must
/// not retroactively change what a player was told they scored. Only the wording (`verdict`,
/// `tip`) is read from the catalog, and it degrades to an empty string if that scenario was
/// retired — the numbers still render.
fn outcome_of(row: &Round, scenario: Option<&Scenario>) -> Result<Outcome, DailyError> {
    let choice = scenario.and_then(|s| s.choice(&row.choice_key));
    Ok(Outcome {
        choice_key: row.choice_key.clone(),
        choice_label: choice.map(|c| c.label.clone()).unwrap_or_default(),
        delta: row.score_delta,
        streak_bonus: row.streak_bonus,
        credit: row.credit_after,
        grade: Grade::of(row.credit_after),
        grade_label: Grade::of(row.credit_after).label(),
        streak: row.streak_after,
        verdict: choice.map(|c| c.verdict.clone()).unwrap_or_default(),
        tip: choice.map(|c| c.tip.clone()).unwrap_or_default(),
    })
}

async fn load_round(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    req: &Player,
    date: NaiveDate,
) -> Result<Option<Round>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT play_date, scenario_id, choice_key, score_delta, streak_bonus,
               credit_after, streak_after
          FROM daily_round
         WHERE player_id = $1 AND campaign_id = $2 AND play_date = $3
        "#,
    )
    .bind(req.player_id)
    .bind(req.campaign_id)
    .bind(date)
    .fetch_optional(&mut **tx)
    .await?;

    row.map(round_from_row).transpose()
}

async fn load_last_round(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    req: &Player,
) -> Result<Option<Round>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT play_date, scenario_id, choice_key, score_delta, streak_bonus,
               credit_after, streak_after
          FROM daily_round
         WHERE player_id = $1 AND campaign_id = $2
         ORDER BY play_date DESC
         LIMIT 1
        "#,
    )
    .bind(req.player_id)
    .bind(req.campaign_id)
    .fetch_optional(&mut **tx)
    .await?;

    row.map(round_from_row).transpose()
}

fn round_from_row(r: sqlx::postgres::PgRow) -> Result<Round, sqlx::Error> {
    Ok(Round {
        play_date: r.try_get("play_date")?,
        scenario_id: r.try_get("scenario_id")?,
        choice_key: r.try_get("choice_key")?,
        score_delta: r.try_get("score_delta")?,
        streak_bonus: r.try_get("streak_bonus")?,
        credit_after: r.try_get("credit_after")?,
        streak_after: r.try_get("streak_after")?,
    })
}

async fn count_rounds(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    req: &Player,
) -> Result<i64, sqlx::Error> {
    sqlx::query("SELECT count(*) AS n FROM daily_round WHERE player_id = $1 AND campaign_id = $2")
        .bind(req.player_id)
        .bind(req.campaign_id)
        .fetch_one(&mut **tx)
        .await?
        .try_get("n")
}

async fn count_players(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    req: &Player,
) -> Result<i64, sqlx::Error> {
    sqlx::query("SELECT count(DISTINCT player_id) AS n FROM daily_round WHERE campaign_id = $1")
        .bind(req.campaign_id)
        .fetch_one(&mut **tx)
        .await?
        .try_get("n")
}

/// Rank among players who have answered, plus the size of the field.
///
/// Rank counts players strictly ahead, so ties share a place. `count(*)` is already `bigint`
/// (unlike `sum()`, which decodes as NUMERIC — see CLAUDE.md), so no cast is needed here.
async fn standing(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    req: &Player,
    credit: i32,
    streak: i32,
) -> Result<Standing, sqlx::Error> {
    let row = sqlx::query(
        r#"
        WITH latest AS (
          SELECT DISTINCT ON (dr.player_id) dr.credit_after, dr.streak_after
            FROM daily_round dr
           WHERE dr.campaign_id = $1
           ORDER BY dr.player_id, dr.play_date DESC, dr.id DESC
        )
        SELECT count(*) AS players,
               count(*) FILTER (
                 WHERE (credit_after, streak_after) > ($2::int, $3::int)
               ) + 1 AS rank
          FROM latest
        "#,
    )
    .bind(req.campaign_id)
    .bind(credit)
    .bind(streak)
    .fetch_one(&mut **tx)
    .await?;

    Ok(Standing {
        rank: row.try_get("rank")?,
        players: row.try_get("players")?,
    })
}

/// Leaderboard display name.
///
/// The Telegram handle is a public identifier and is shown as-is; `tg_user_id` — the key the whole
/// attribution and risk chain is built on — is never part of this response. Players without a
/// handle get a stable pseudonym rather than being dropped from the board.
fn display_name(username: Option<String>, player_id: i64) -> String {
    match username {
        Some(u) if !u.trim().is_empty() => u,
        _ => format!("玩家 #{:04}", player_id % 10_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daily::{Choice, Scenario};

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    fn catalog() -> Vec<Scenario> {
        vec![Scenario {
            id: 7,
            code: "flash_sale".into(),
            title: "直播间秒杀".into(),
            prompt: "…".into(),
            choices: vec![Choice {
                key: "skip".into(),
                label: "不买".into(),
                delta: 10,
                verdict: "对。".into(),
                tip: "折扣不会让不需要的东西变成需要。".into(),
            }],
        }]
    }

    fn round() -> Round {
        Round {
            play_date: date(2026, 8, 4),
            scenario_id: 7,
            choice_key: "skip".into(),
            score_delta: 10,
            streak_bonus: 5,
            credit_after: 712,
            streak_after: 7,
        }
    }

    /// The stored numbers are what the player was shown; re-scoring against the current catalog
    /// would let a content edit rewrite a past result.
    #[test]
    fn replayed_outcome_uses_stored_scores_not_the_current_catalog() {
        let mut edited = catalog();
        edited[0].choices[0].delta = -99;

        let out = outcome_of(&round(), find_scenario(&edited, 7)).expect("builds");
        assert_eq!(out.delta, 10);
        assert_eq!(out.streak_bonus, 5);
        assert_eq!(out.credit, 712);
        assert_eq!(out.grade, Grade::Strong);
        // Wording still comes from the catalog.
        assert_eq!(out.choice_label, "不买");
        assert!(out.tip.contains("折扣"));
    }

    /// A retired scenario must still render the player's history rather than 500.
    #[test]
    fn replay_survives_a_scenario_that_left_the_catalog() {
        let out = outcome_of(&round(), None).expect("builds");
        assert_eq!(out.credit, 712);
        assert_eq!(out.streak, 7);
        assert!(out.verdict.is_empty() && out.tip.is_empty());
    }

    #[test]
    fn players_without_a_telegram_handle_still_get_a_board_name() {
        assert_eq!(display_name(Some("demo_user".into()), 1), "demo_user");
        assert_eq!(display_name(Some("  ".into()), 42), "玩家 #0042");
        assert_eq!(display_name(None, 123_456), "玩家 #3456");
    }

    #[test]
    fn scenario_rotation_is_bounded_by_the_catalog() {
        let cat = catalog();
        let req = Player {
            tenant_id: 1,
            player_id: 1,
            campaign_id: 2,
            date: date(2026, 8, 4),
        };
        assert_eq!(pick_scenario(&cat, &req).expect("picks").id, 7);
        assert!(matches!(
            pick_scenario(&[], &req),
            Err(DailyError::NoScenario)
        ));
    }
}
