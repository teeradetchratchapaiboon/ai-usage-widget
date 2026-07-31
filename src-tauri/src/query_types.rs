use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Time range for querying usage history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// Granularity for aggregating usage data in history queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    Hourly,
    Daily,
    Weekly,
    Monthly,
}

impl Granularity {
    /// Returns the SQLite strftime format string for time bucketing.
    pub fn strftime_format(&self) -> &'static str {
        match self {
            Granularity::Hourly => "%Y-%m-%dT%H:00:00Z",
            Granularity::Daily => "%Y-%m-%dT00:00:00Z",
            Granularity::Weekly => "%Y-W%W",
            Granularity::Monthly => "%Y-%m-01T00:00:00Z",
        }
    }
}

/// Aggregated usage record for a time bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub timestamp: String,
    pub provider_id: String,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub quota_fast_pct: Option<f64>,
    pub quota_standard_pct: Option<f64>,
}

/// Summary of current usage across all providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSummary {
    pub providers: Vec<ProviderUsageSummary>,
    pub total_tokens_today: Option<i64>,
    pub total_tokens_this_week: Option<i64>,
    pub last_updated: DateTime<Utc>,
}

/// Per-provider usage summary for the current period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderUsageSummary {
    pub provider_id: String,
    pub input_tokens_today: Option<i64>,
    pub output_tokens_today: Option<i64>,
    pub total_tokens_today: Option<i64>,
    pub input_tokens_this_week: Option<i64>,
    pub output_tokens_this_week: Option<i64>,
    pub total_tokens_this_week: Option<i64>,
    pub last_activity: Option<String>,
}
