//! Growth analytics and settlement reporting module.

pub mod dashboard;

pub use dashboard::{
    CampaignMetrics, KolPerformanceMetrics, TenantGrowthMetrics, GrowthAnalyticsService,
};
