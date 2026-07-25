//! Probabilistic and multi-channel non-deterministic attribution matching engine.
//!
//! Evaluates non-deterministic touchpoints (device fingerprints, IP + UA, time windows, clipboard)
//! and multi-touch channel weighting decay models (First-Touch, Last-Touch, Position-Based, Linear).
//!
//! **Constraint C1 Compliance**: Non-deterministic matching yields `is_billable = false` and
//! confidence < 100. They populate analytics dashboards and multi-channel marketing views,
//! but NEVER enter billing streams or generate invoices.

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::models::{AttributionMethod, Cents};

/// Touchpoint captured from a marketing channel click / impression.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Touchpoint {
    pub id: i64,
    pub channel: String,
    pub campaign_id: i64,
    pub kol_id: i64,
    pub ip_hash: String,
    pub ua_hash: String,
    pub touched_at: DateTime<Utc>,
    pub click_payload: Option<serde_json::Value>,
}

/// Device fingerprint collected during app first launch or web landing.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceFingerprint {
    pub ip_hash: String,
    pub ua_hash: String,
    pub locale: String,
    pub screen_res: String,
    pub captured_at: DateTime<Utc>,
}

/// Multi-touch attribution decay model type.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiTouchModel {
    /// 100% weight to the last touchpoint prior to conversion.
    LastTouch,
    /// 100% weight to the first touchpoint in the click window.
    FirstTouch,
    /// Equal weight distributed across all touchpoints in the click window.
    Linear,
    /// 40% First Touch, 40% Last Touch, 20% distributed among middle touches.
    PositionBased,
}

/// Output of a multi-channel attribution calculation.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiTouchAttributionResult {
    pub touchpoint_id: i64,
    pub kol_id: i64,
    pub campaign_id: i64,
    pub channel: String,
    pub weight: f64,
    pub allocated_cents: Cents,
}

/// Candidate match outcome from non-deterministic matching.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbabilisticMatch {
    pub touchpoint_id: i64,
    pub kol_id: i64,
    pub campaign_id: i64,
    pub method: AttributionMethod,
    pub confidence: i16,
    pub is_billable: bool,
    pub matched_at: DateTime<Utc>,
    pub score: f64,
}

/// Non-deterministic matcher implementation.
pub struct ProbabilisticMatcher {
    /// Maximum lookback window for fingerprint matching (default: 24h).
    pub max_window: TimeDelta,
}

impl Default for ProbabilisticMatcher {
    fn default() -> Self {
        Self {
            max_window: TimeDelta::hours(24),
        }
    }
}

impl ProbabilisticMatcher {
    #[allow(dead_code)]
    pub fn new(max_window: TimeDelta) -> Self {
        Self { max_window }
    }

    /// Match a device fingerprint against a list of recent touchpoints.
    pub fn match_fingerprint(
        &self,
        fp: &DeviceFingerprint,
        touches: &[Touchpoint],
    ) -> Option<ProbabilisticMatch> {
        let mut best: Option<(Touchpoint, f64)> = None;

        for touch in touches {
            if touch.touched_at > fp.captured_at {
                continue; // Cannot match touch after launch
            }

            let delta = fp.captured_at.signed_duration_since(touch.touched_at);
            if delta > self.max_window {
                continue;
            }

            let mut score = 0.0;
            if !fp.ip_hash.is_empty() && fp.ip_hash == touch.ip_hash {
                score += 0.6;
            }
            if !fp.ua_hash.is_empty() && fp.ua_hash == touch.ua_hash {
                score += 0.3;
            }

            // Recency decay factor (closer to launch = higher score)
            let recency_factor = 1.0
                - (delta.num_seconds() as f64 / self.max_window.num_seconds() as f64)
                    .clamp(0.0, 1.0);
            score += 0.1 * recency_factor;

            if score >= 0.5 {
                if let Some((_, best_score)) = &best {
                    if score > *best_score {
                        best = Some((touch.clone(), score));
                    }
                } else {
                    best = Some((touch.clone(), score));
                }
            }
        }

        best.map(|(touch, score)| ProbabilisticMatch {
            touchpoint_id: touch.id,
            kol_id: touch.kol_id,
            campaign_id: touch.campaign_id,
            method: AttributionMethod::Probabilistic,
            confidence: AttributionMethod::Probabilistic.confidence(),
            is_billable: AttributionMethod::Probabilistic.is_billable(), // ALWAYS false (C1)
            matched_at: fp.captured_at,
            score,
        })
    }

    /// Calculate multi-touch channel weighting decay across a user's journey.
    pub fn calculate_multi_touch(
        &self,
        touches: &[Touchpoint],
        total_amount: Cents,
        model: MultiTouchModel,
    ) -> Vec<MultiTouchAttributionResult> {
        if touches.is_empty() {
            return vec![];
        }

        let n = touches.len();
        let weights: Vec<f64> = match model {
            MultiTouchModel::LastTouch => {
                let mut w = vec![0.0; n];
                w[n - 1] = 1.0;
                w
            }
            MultiTouchModel::FirstTouch => {
                let mut w = vec![0.0; n];
                w[0] = 1.0;
                w
            }
            MultiTouchModel::Linear => vec![1.0 / n as f64; n],
            MultiTouchModel::PositionBased => {
                if n == 1 {
                    vec![1.0]
                } else if n == 2 {
                    vec![0.5, 0.5]
                } else {
                    let middle_count = (n - 2) as f64;
                    let middle_weight = 0.2 / middle_count;
                    let mut w = vec![middle_weight; n];
                    w[0] = 0.4;
                    w[n - 1] = 0.4;
                    w
                }
            }
        };

        touches
            .iter()
            .zip(weights.iter())
            .map(|(t, &w)| {
                let allocated = Cents((total_amount.0 as f64 * w).round() as i64);
                MultiTouchAttributionResult {
                    touchpoint_id: t.id,
                    kol_id: t.kol_id,
                    campaign_id: t.campaign_id,
                    channel: t.channel.clone(),
                    weight: w,
                    allocated_cents: allocated,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probabilistic_match_is_never_billable() {
        let matcher = ProbabilisticMatcher::default();
        let now = Utc::now();

        let fp = DeviceFingerprint {
            ip_hash: "ip_123".into(),
            ua_hash: "ua_abc".into(),
            locale: "zh_CN".into(),
            screen_res: "1170x2532".into(),
            captured_at: now,
        };

        let touch = Touchpoint {
            id: 100,
            channel: "telegram_channel_a".into(),
            campaign_id: 1,
            kol_id: 42,
            ip_hash: "ip_123".into(),
            ua_hash: "ua_abc".into(),
            touched_at: now - TimeDelta::minutes(30),
            click_payload: None,
        };

        let res = matcher.match_fingerprint(&fp, &[touch]).unwrap();
        assert!(
            !res.is_billable,
            "Probabilistic match MUST NOT be billable per constraint C1"
        );
        assert_eq!(res.method, AttributionMethod::Probabilistic);
        assert_eq!(res.confidence, 30);
        assert_eq!(res.kol_id, 42);
        assert!(res.score >= 0.9);
    }

    #[test]
    fn multi_touch_linear_allocates_proportionally() {
        let matcher = ProbabilisticMatcher::default();
        let now = Utc::now();

        let touches = vec![
            Touchpoint {
                id: 1,
                channel: "x_twitter".into(),
                campaign_id: 1,
                kol_id: 10,
                ip_hash: "ip_1".into(),
                ua_hash: "ua_1".into(),
                touched_at: now - TimeDelta::hours(5),
                click_payload: None,
            },
            Touchpoint {
                id: 2,
                channel: "telegram".into(),
                campaign_id: 1,
                kol_id: 11,
                ip_hash: "ip_1".into(),
                ua_hash: "ua_1".into(),
                touched_at: now - TimeDelta::hours(1),
                click_payload: None,
            },
        ];

        let results =
            matcher.calculate_multi_touch(&touches, Cents(10000), MultiTouchModel::Linear);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].allocated_cents, Cents(5000));
        assert_eq!(results[1].allocated_cents, Cents(5000));
    }

    #[test]
    fn multi_touch_position_based_allocates_40_20_40() {
        let matcher = ProbabilisticMatcher::default();
        let now = Utc::now();

        let touches = vec![
            Touchpoint {
                id: 1,
                channel: "c1".into(),
                campaign_id: 1,
                kol_id: 10,
                ip_hash: "ip_1".into(),
                ua_hash: "ua_1".into(),
                touched_at: now - TimeDelta::hours(10),
                click_payload: None,
            },
            Touchpoint {
                id: 2,
                channel: "c2".into(),
                campaign_id: 1,
                kol_id: 11,
                ip_hash: "ip_1".into(),
                ua_hash: "ua_1".into(),
                touched_at: now - TimeDelta::hours(5),
                click_payload: None,
            },
            Touchpoint {
                id: 3,
                channel: "c3".into(),
                campaign_id: 1,
                kol_id: 12,
                ip_hash: "ip_1".into(),
                ua_hash: "ua_1".into(),
                touched_at: now - TimeDelta::hours(1),
                click_payload: None,
            },
        ];

        let results =
            matcher.calculate_multi_touch(&touches, Cents(10000), MultiTouchModel::PositionBased);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].allocated_cents, Cents(4000)); // 40% First
        assert_eq!(results[1].allocated_cents, Cents(2000)); // 20% Middle
        assert_eq!(results[2].allocated_cents, Cents(4000)); // 40% Last
    }
}
