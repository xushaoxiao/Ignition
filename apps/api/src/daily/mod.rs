//! Daily budget-decision game — scoring, streaks, and the virtual credit score.
//!
//! The five wheel-style games are animation skins over one server-decided prize draw. This one is
//! a different shape: the player picks an answer, the **server** scores it, and the score
//! accumulates across days. What stays identical is the rule that matters — the client never
//! decides an outcome. It posts a choice key and receives a verdict; the score table lives here
//! and is never serialised to the client.
//!
//! It is deliberately an **engagement layer, not a billing input**. Streaks, credit score, and the
//! leaderboard exist to drive return visits; the only billable event is still the claim code the
//! player redeems in the main app (constraint C1). Nothing in this module writes a
//! `billable_event`, and it must stay that way — a score the player can influence must never move
//! an invoice.
//!
//! All logic here is pure and time-injected (`date`, never `Utc::today()`), so "does a streak
//! survive a gap over a month boundary" is a unit test rather than a wait.

pub mod round;

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

/// Starting credit score for a player who has never answered.
pub const CREDIT_START: i32 = 600;
/// Floor and ceiling. A player who answers badly for a month should feel it but never hit a dead
/// end — a score that cannot recover is a player who does not come back.
pub const CREDIT_MIN: i32 = 300;
pub const CREDIT_MAX: i32 = 850;

/// One scored option of a scenario.
///
/// `delta` is the whole game. It is `Deserialize`-only and **never** `Serialize`: the public
/// projection is [`PublicChoice`] (key + label). Same rule as the prize pool, where `Segment`
/// exposes id and label but never weight or stock — publishing the score table turns a decision
/// game into a lookup table, and the leaderboard into a copy-paste exercise.
#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    pub key: String,
    pub label: String,
    /// Credit-score change, roughly -15..=+15.
    pub delta: i32,
    /// Immediate feedback on the decision.
    pub verdict: String,
    /// The teaching line — interest cost, credit impact, opportunity cost.
    pub tip: String,
}

/// One day's scenario, as stored in the `daily_scenario` catalog.
#[derive(Debug, Clone)]
pub struct Scenario {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub prompt: String,
    pub choices: Vec<Choice>,
}

impl Scenario {
    /// Look up a submitted choice key. Unknown keys are rejected rather than defaulted — a client
    /// sending a key we do not recognise is either stale or probing, and awarding it any score
    /// would be inventing one.
    pub fn choice(&self, key: &str) -> Option<&Choice> {
        self.choices.iter().find(|c| c.key == key)
    }
}

/// Client-facing projection of a choice: no `delta`, no `verdict`, no `tip`.
///
/// Feedback text is withheld until after the answer on purpose — it names the better option, so
/// shipping it up front answers the question for the player.
#[derive(Debug, Clone, Serialize)]
pub struct PublicChoice {
    pub key: String,
    pub label: String,
}

/// Client-facing projection of a scenario.
#[derive(Debug, Clone, Serialize)]
pub struct PublicScenario {
    pub id: i64,
    pub code: String,
    pub title: String,
    pub prompt: String,
    pub choices: Vec<PublicChoice>,
}

impl From<&Scenario> for PublicScenario {
    fn from(s: &Scenario) -> Self {
        PublicScenario {
            id: s.id,
            code: s.code.clone(),
            title: s.title.clone(),
            prompt: s.prompt.clone(),
            choices: s
                .choices
                .iter()
                .map(|c| PublicChoice {
                    key: c.key.clone(),
                    label: c.label.clone(),
                })
                .collect(),
        }
    }
}

/// Credit-score tier shown next to the number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Grade {
    Building,
    Steady,
    Strong,
    Excellent,
}

impl Grade {
    pub fn of(credit: i32) -> Grade {
        match credit {
            c if c >= 740 => Grade::Excellent,
            c if c >= 670 => Grade::Strong,
            c if c >= 580 => Grade::Steady,
            _ => Grade::Building,
        }
    }

    /// Display label. Chinese because the TMA copy is Chinese; when the catalog gains other
    /// locales this moves to a lookup keyed by `daily_scenario.locale`.
    pub fn label(&self) -> &'static str {
        match self {
            Grade::Building => "起步",
            Grade::Steady => "稳健",
            Grade::Strong => "优秀",
            Grade::Excellent => "极佳",
        }
    }
}

/// Which scenario a campaign shows on a given date.
///
/// Deterministic on purpose, and identical for every player in the campaign: a daily challenge
/// with a leaderboard is only fair if everyone answered the same question that day. Randomising
/// per player would also break the whole point of the format — there is nothing to talk about in
/// the group chat if nobody got the same scenario.
///
/// `campaign_id` shifts the phase so two campaigns starting the same week do not march through the
/// catalog in lockstep.
///
/// The index is into the catalog ordered by `id`, so scenarios must be appended, never renumbered
/// — same constraint as the prize pool's `ORDER BY id`. History is unaffected either way:
/// `daily_round` stores the resolved `scenario_id`.
pub fn scenario_index(count: usize, campaign_id: i64, date: NaiveDate) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let day = i64::from(date.num_days_from_ce());
    Some((day.wrapping_add(campaign_id).rem_euclid(count as i64)) as usize)
}

/// Streak after answering on `today`, given the previous round's date and streak.
///
/// Exactly one calendar day of gap keeps the chain; anything longer restarts at 1. Missing a day
/// resets rather than decays because the recovery has to be simple enough to explain in one line
/// ("连续第 N 天") — a decaying counter nobody can predict is not a habit loop.
pub fn advance_streak(previous: Option<(NaiveDate, i32)>, today: NaiveDate) -> i32 {
    match previous {
        None => 1,
        // Same-day replay: the unique index on (tenant, player, campaign, play_date) makes this
        // unreachable through `answer`, but returning the stored streak keeps a replay idempotent
        // instead of inflating the counter.
        Some((last, streak)) if last == today => streak.max(1),
        Some((last, streak)) if last.succ_opt() == Some(today) => streak.saturating_add(1),
        Some(_) => 1,
    }
}

/// Extra credit for showing up. This is the return-visit mechanic, so it is worth points that a
/// single good decision cannot match: a perfect answer is +14, a seven-day chain is +5 on top.
pub fn streak_bonus(streak: i32) -> i32 {
    match streak {
        s if s > 0 && s % 7 == 0 => 5,
        s if s >= 3 => 2,
        _ => 0,
    }
}

/// Apply a score change, clamped to the score range.
pub fn apply_delta(credit: i32, delta: i32) -> i32 {
    credit.saturating_add(delta).clamp(CREDIT_MIN, CREDIT_MAX)
}

/// Soft prompt shown to high scorers, read from `campaign.config.promo`.
///
/// Deliberately campaign configuration and not scenario content: the scenario catalog is the
/// game's own voice and stays neutral about products, while this line is the customer's own claim
/// under their own name. Mixing the two would make a marketing message read as the game's advice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Promo {
    pub text: String,
    #[serde(default)]
    pub url: Option<String>,
    /// Credit score at which the prompt appears. Defaults to "strong" so it reaches players who
    /// have actually played, not first-time visitors.
    #[serde(default = "default_min_credit")]
    pub min_credit: i32,
}

fn default_min_credit() -> i32 {
    670
}

impl Promo {
    pub fn visible_at(&self, credit: i32) -> bool {
        credit >= self.min_credit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    // ---------------------------------------------------------------- scenario rotation

    /// The leaderboard only means something if the day's question is the same for everyone.
    #[test]
    fn every_player_in_a_campaign_gets_the_same_scenario_that_day() {
        let d = date(2026, 8, 4);
        let a = scenario_index(12, 7, d);
        for _ in 0..10 {
            assert_eq!(
                scenario_index(12, 7, d),
                a,
                "index must not depend on the caller"
            );
        }
    }

    #[test]
    fn rotation_advances_daily_and_covers_the_whole_catalog() {
        let count = 12;
        let seen: std::collections::HashSet<usize> = (0..count as i64)
            .map(|i| {
                scenario_index(count, 1, date(2026, 8, 4) + chrono::Duration::days(i))
                    .expect("catalog is not empty")
            })
            .collect();
        assert_eq!(
            seen.len(),
            count,
            "consecutive days must not repeat within one cycle"
        );
    }

    /// Two campaigns launched the same week should not march through the catalog in lockstep.
    #[test]
    fn campaigns_are_out_of_phase() {
        let d = date(2026, 8, 4);
        assert_ne!(scenario_index(12, 1, d), scenario_index(12, 2, d));
    }

    #[test]
    fn empty_catalog_yields_nothing() {
        assert_eq!(scenario_index(0, 1, date(2026, 8, 4)), None);
    }

    /// A far-past date must wrap, not panic or go negative.
    #[test]
    fn indices_stay_in_range_across_a_wide_date_span() {
        for years in [-100i64, -1, 0, 1, 50] {
            let d = date(2026, 8, 4) + chrono::Duration::days(years * 365);
            let i = scenario_index(12, -3, d).expect("catalog is not empty");
            assert!(i < 12, "index {i} out of range for {d}");
        }
    }

    // ---------------------------------------------------------------- streaks

    #[test]
    fn first_ever_round_starts_the_streak() {
        assert_eq!(advance_streak(None, date(2026, 8, 4)), 1);
    }

    #[test]
    fn consecutive_days_extend_the_streak() {
        assert_eq!(
            advance_streak(Some((date(2026, 8, 3), 4)), date(2026, 8, 4)),
            5
        );
    }

    /// Month and year boundaries are the ones that break naive day-number arithmetic.
    #[test]
    fn streak_survives_month_and_year_boundaries() {
        assert_eq!(
            advance_streak(Some((date(2026, 7, 31), 9)), date(2026, 8, 1)),
            10
        );
        assert_eq!(
            advance_streak(Some((date(2026, 12, 31), 30)), date(2027, 1, 1)),
            31
        );
        // 2028 is a leap year.
        assert_eq!(
            advance_streak(Some((date(2028, 2, 28), 2)), date(2028, 2, 29)),
            3
        );
    }

    #[test]
    fn a_missed_day_resets_the_streak() {
        assert_eq!(
            advance_streak(Some((date(2026, 8, 2), 20)), date(2026, 8, 4)),
            1
        );
    }

    /// Same-day replay must not inflate the counter — replays of one answer are idempotent.
    #[test]
    fn answering_twice_on_the_same_day_does_not_extend_the_streak() {
        assert_eq!(
            advance_streak(Some((date(2026, 8, 4), 6)), date(2026, 8, 4)),
            6
        );
    }

    // ---------------------------------------------------------------- scoring

    #[test]
    fn streak_bonus_pays_at_three_days_and_every_seventh() {
        for (streak, want) in [
            (1, 0),
            (2, 0),
            (3, 2),
            (6, 2),
            (7, 5),
            (8, 2),
            (14, 5),
            (0, 0),
        ] {
            assert_eq!(streak_bonus(streak), want, "streak={streak}");
        }
    }

    #[test]
    fn credit_is_clamped_to_the_score_range() {
        assert_eq!(apply_delta(CREDIT_MAX - 2, 12), CREDIT_MAX);
        assert_eq!(apply_delta(CREDIT_MIN + 2, -12), CREDIT_MIN);
        assert_eq!(apply_delta(CREDIT_START, 12), CREDIT_START + 12);
    }

    /// Clamping must not be reachable around an overflow.
    #[test]
    fn extreme_deltas_cannot_escape_the_range() {
        assert_eq!(apply_delta(CREDIT_START, i32::MAX), CREDIT_MAX);
        assert_eq!(apply_delta(CREDIT_START, i32::MIN), CREDIT_MIN);
    }

    #[test]
    fn grade_boundaries_are_inclusive_at_the_lower_edge() {
        for (credit, want) in [
            (CREDIT_MIN, Grade::Building),
            (579, Grade::Building),
            (580, Grade::Steady),
            (669, Grade::Steady),
            (670, Grade::Strong),
            (739, Grade::Strong),
            (740, Grade::Excellent),
            (CREDIT_MAX, Grade::Excellent),
        ] {
            assert_eq!(Grade::of(credit), want, "credit={credit}");
        }
    }

    // ---------------------------------------------------------------- projection

    fn scenario() -> Scenario {
        Scenario {
            id: 2,
            code: "new_phone".into(),
            title: "想换新手机".into(),
            prompt: "…".into(),
            choices: vec![
                Choice {
                    key: "wait_save".into(),
                    label: "再存两个月".into(),
                    delta: 12,
                    verdict: "最稳".into(),
                    tip: "应急储备的作用是…".into(),
                },
                Choice {
                    key: "cash_loan".into(),
                    label: "借短期现金贷".into(),
                    delta: -14,
                    verdict: "最贵的选项".into(),
                    tip: "年化常在 24%–36%".into(),
                },
            ],
        }
    }

    /// The client must be able to render the question without being able to read the answer.
    #[test]
    fn public_projection_carries_no_scoring_information() {
        let public = PublicScenario::from(&scenario());
        let json = serde_json::to_string(&public).expect("serialises");
        for leak in ["delta", "verdict", "tip", "12", "-14", "最稳", "年化"] {
            assert!(
                !json.contains(leak),
                "public scenario leaked {leak:?}: {json}"
            );
        }
        assert!(json.contains("wait_save") && json.contains("再存两个月"));
    }

    #[test]
    fn unknown_choice_keys_are_rejected_rather_than_defaulted() {
        let s = scenario();
        assert!(s.choice("wait_save").is_some());
        assert!(s.choice("").is_none());
        assert!(s.choice("WAIT_SAVE").is_none());
        assert!(s.choice("some_key_a_stale_client_sent").is_none());
    }

    // ---------------------------------------------------------------- promo

    #[test]
    fn promo_is_withheld_below_its_threshold_and_defaults_to_strong() {
        let promo: Promo = serde_json::from_str(r#"{"text":"关注官方频道"}"#).expect("parses");
        assert_eq!(promo.min_credit, 670);
        assert!(!promo.visible_at(669));
        assert!(promo.visible_at(670));
    }
}
