//! Tenant and KOL growth analytics & settlement dashboard reporting logic.

use serde::{Deserialize, Serialize};
use crate::models::Cents;

/// Overview of tenant growth metrics and commercial performance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantGrowthMetrics {
    pub tenant_id: i64,
    pub period_start: String,
    pub period_end: String,
    pub total_clicks: i64,
    pub total_conversions: i64,
    pub billable_conversions: i64,
    pub non_billable_conversions: i64,
    pub conversion_rate_pct: f64,
    pub total_gmv_cents: Cents,
    pub platform_fee_cents: Cents,
    pub kol_payout_cents: Cents,
}

/// Performance and settlement status for an individual KOL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KolPerformanceMetrics {
    pub kol_id: i64,
    pub kol_name: String,
    pub total_attributed_users: i64,
    pub billable_conversions: i64,
    pub pending_payable_cents: Cents,
    pub cleared_payable_cents: Cents,
    pub invoiced_payable_cents: Cents,
}

/// Campaign level ROI and channel performance breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignMetrics {
    pub campaign_id: i64,
    pub campaign_name: String,
    pub channel: String,
    pub clicks: i64,
    pub conversions: i64,
    pub spend_cents: Cents,
    pub gmv_cents: Cents,
    pub roi: f64,
}

/// Analytics service helper.
pub struct GrowthAnalyticsService;

impl GrowthAnalyticsService {
    /// Calculate growth metrics summary.
    pub fn summarize_tenant_growth(
        tenant_id: i64,
        period_start: &str,
        period_end: &str,
        total_clicks: i64,
        billable_conversions: i64,
        non_billable_conversions: i64,
        total_gmv_cents: Cents,
        platform_fee_cents: Cents,
        kol_payout_cents: Cents,
    ) -> TenantGrowthMetrics {
        let total_conversions = billable_conversions + non_billable_conversions;
        let conversion_rate_pct = if total_clicks > 0 {
            (total_conversions as f64 / total_clicks as f64) * 100.0
        } else {
            0.0
        };

        TenantGrowthMetrics {
            tenant_id,
            period_start: period_start.to_string(),
            period_end: period_end.to_string(),
            total_clicks,
            total_conversions,
            billable_conversions,
            non_billable_conversions,
            conversion_rate_pct,
            total_gmv_cents,
            platform_fee_cents,
            kol_payout_cents,
        }
    }

    /// Calculate ROI for a campaign.
    pub fn calculate_campaign_roi(
        campaign_id: i64,
        campaign_name: &str,
        channel: &str,
        clicks: i64,
        conversions: i64,
        spend_cents: Cents,
        gmv_cents: Cents,
    ) -> CampaignMetrics {
        let roi = if spend_cents.0 > 0 {
            (gmv_cents.0 as f64 - spend_cents.0 as f64) / spend_cents.0 as f64
        } else {
            0.0
        };

        CampaignMetrics {
            campaign_id,
            campaign_name: campaign_name.to_string(),
            channel: channel.to_string(),
            clicks,
            conversions,
            spend_cents,
            gmv_cents,
            roi,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_tenant_growth_calculates_conversion_rate() {
        let summary = GrowthAnalyticsService::summarize_tenant_growth(
            1,
            "2026-07-01",
            "2026-07-31",
            1000,
            50,
            10,
            Cents(500000),
            Cents(25000),
            Cents(100000),
        );

        assert_eq!(summary.total_conversions, 60);
        assert_eq!(summary.conversion_rate_pct, 6.0);
        assert_eq!(summary.platform_fee_cents, Cents(25000));
    }

    #[test]
    fn campaign_roi_handles_zero_spend() {
        let metrics = GrowthAnalyticsService::calculate_campaign_roi(
            101,
            "Summer Growth",
            "telegram",
            500,
            25,
            Cents(0),
            Cents(10000),
        );

        assert_eq!(metrics.roi, 0.0);
    }

    #[test]
    fn campaign_roi_calculates_positive_return() {
        let metrics = GrowthAnalyticsService::calculate_campaign_roi(
            102,
            "KOL Blitz",
            "x_twitter",
            2000,
            100,
            Cents(100000), // Spend 1,000 USD
            Cents(300000), // GMV 3,000 USD -> Profit 2,000 -> ROI 2.0
        );

        assert_eq!(metrics.roi, 2.0);
    }
}
