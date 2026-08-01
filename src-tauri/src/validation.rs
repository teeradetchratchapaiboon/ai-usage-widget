use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::error::ValidationError;
use crate::query_types::{Granularity, TimeRange};

/// Maximum allowed time range in days.
const MAX_RANGE_DAYS: i64 = 366;

/// Maximum allowed future tolerance for the end timestamp (1 hour).
const MAX_FUTURE_TOLERANCE_HOURS: i64 = 1;

/// Minimum collection interval in seconds.
const MIN_COLLECTION_INTERVAL_SECS: u32 = 10;

/// Maximum collection interval in seconds.
const MAX_COLLECTION_INTERVAL_SECS: u32 = 3600;

/// Minimum retention days.
const MIN_RETENTION_DAYS: u32 = 1;

/// Maximum retention days.
const MAX_RETENTION_DAYS: u32 = 3650;

/// Allowed locale values.
const ALLOWED_LOCALES: &[&str] = &["th", "en"];

/// Partial settings for updates via IPC. Only fields that are `Some` will be applied.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PartialSettings {
    pub locale: Option<String>,
    pub collection_interval_secs: Option<u32>,
    pub retention_days: Option<u32>,
    pub always_on_top: Option<bool>,
    pub click_through: Option<bool>,
    pub autostart: Option<bool>,
    pub notification_warning_pct: Option<f64>,
    pub notification_critical_pct: Option<f64>,
}

/// Validate a time range for usage history queries.
///
/// Rejects if:
/// - start >= end
/// - range exceeds 366 days
/// - end is more than 1 hour in the future
pub fn validate_time_range(range: &TimeRange) -> Result<(), ValidationError> {
    // Reject if start >= end
    if range.start >= range.end {
        return Err(ValidationError::InvalidRange(
            "start must be before end".to_string(),
        ));
    }

    // Reject if range > 366 days
    let duration = range.end - range.start;
    let max_duration = Duration::days(MAX_RANGE_DAYS);
    if duration > max_duration {
        let range_days = duration.num_days() as u32;
        return Err(ValidationError::RangeTooLarge(
            range_days,
            MAX_RANGE_DAYS as u32,
        ));
    }

    // Reject if end is more than 1 hour in the future
    let now = Utc::now();
    let max_future = now + Duration::hours(MAX_FUTURE_TOLERANCE_HOURS);
    if range.end > max_future {
        return Err(ValidationError::FutureDate(format!(
            "end timestamp {} is more than 1 hour in the future",
            range.end
        )));
    }

    Ok(())
}

/// Validate partial settings for an update operation.
///
/// Rejects if:
/// - locale is not "th" or "en"
/// - collection_interval_secs is outside 10-3600
/// - retention_days is outside 1-3650
pub fn validate_settings(settings: &PartialSettings) -> Result<(), ValidationError> {
    // Validate locale if provided
    if let Some(ref locale) = settings.locale {
        if !ALLOWED_LOCALES.contains(&locale.as_str()) {
            return Err(ValidationError::InvalidLocale(format!(
                "'{}' is not supported, allowed values: {:?}",
                locale, ALLOWED_LOCALES
            )));
        }
    }

    // Validate collection interval if provided
    if let Some(interval) = settings.collection_interval_secs {
        if !(MIN_COLLECTION_INTERVAL_SECS..=MAX_COLLECTION_INTERVAL_SECS).contains(&interval) {
            return Err(ValidationError::InvalidInterval(
                interval,
                MIN_COLLECTION_INTERVAL_SECS,
                MAX_COLLECTION_INTERVAL_SECS,
            ));
        }
    }

    // Validate retention days if provided
    if let Some(days) = settings.retention_days {
        if !(MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(&days) {
            return Err(ValidationError::InvalidRetentionDays(
                days,
                MIN_RETENTION_DAYS,
                MAX_RETENTION_DAYS,
            ));
        }
    }

    // Validate notification thresholds if provided: both must be percentages
    // and warning must stay below critical.
    for (label, value) in [
        ("warning", settings.notification_warning_pct),
        ("critical", settings.notification_critical_pct),
    ] {
        if let Some(pct) = value {
            if !(0.0..=100.0).contains(&pct) || pct.is_nan() {
                return Err(ValidationError::InvalidThreshold(format!(
                    "{} threshold must be between 0 and 100, got {}",
                    label, pct
                )));
            }
        }
    }

    if let (Some(warning), Some(critical)) = (
        settings.notification_warning_pct,
        settings.notification_critical_pct,
    ) {
        if warning >= critical {
            return Err(ValidationError::InvalidThreshold(format!(
                "warning threshold ({}) must be below critical threshold ({})",
                warning, critical
            )));
        }
    }

    Ok(())
}

/// Validate a granularity value.
///
/// Always valid since the enum covers all cases, but included for completeness
/// and to serve as a validation checkpoint in the IPC pipeline.
pub fn validate_granularity(_granularity: &Granularity) -> Result<(), ValidationError> {
    // Granularity is an enum — all variants are valid by construction.
    Ok(())
}

/// Validate a provider ID string.
///
/// Rejects if:
/// - The string is empty
/// - Contains characters other than alphanumeric, hyphens, or underscores
pub fn validate_provider_id(provider_id: &str) -> Result<(), ValidationError> {
    if provider_id.is_empty() {
        return Err(ValidationError::InvalidProviderId(
            "provider_id must not be empty".to_string(),
        ));
    }

    if !provider_id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ValidationError::InvalidProviderId(format!(
            "'{}' contains invalid characters; only alphanumeric, hyphens, and underscores are allowed",
            provider_id
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    // ─── validate_time_range ───────────────────────────────────────────

    #[test]
    fn test_valid_time_range() {
        let now = Utc::now();
        let range = TimeRange {
            start: now - Duration::hours(24),
            end: now,
        };
        assert!(validate_time_range(&range).is_ok());
    }

    #[test]
    fn test_time_range_start_equals_end() {
        let now = Utc::now();
        let range = TimeRange {
            start: now,
            end: now,
        };
        assert_eq!(
            validate_time_range(&range),
            Err(ValidationError::InvalidRange(
                "start must be before end".to_string()
            ))
        );
    }

    #[test]
    fn test_time_range_start_after_end() {
        let now = Utc::now();
        let range = TimeRange {
            start: now,
            end: now - Duration::hours(1),
        };
        assert_eq!(
            validate_time_range(&range),
            Err(ValidationError::InvalidRange(
                "start must be before end".to_string()
            ))
        );
    }

    #[test]
    fn test_time_range_exceeds_max_days() {
        let now = Utc::now();
        let range = TimeRange {
            start: now - Duration::days(400),
            end: now,
        };
        let result = validate_time_range(&range);
        assert!(matches!(
            result,
            Err(ValidationError::RangeTooLarge(400, 366))
        ));
    }

    #[test]
    fn test_time_range_exactly_366_days_is_valid() {
        let now = Utc::now();
        let range = TimeRange {
            start: now - Duration::days(366),
            end: now,
        };
        assert!(validate_time_range(&range).is_ok());
    }

    #[test]
    fn test_time_range_end_far_in_future() {
        let now = Utc::now();
        let range = TimeRange {
            start: now,
            end: now + Duration::hours(2),
        };
        let result = validate_time_range(&range);
        assert!(matches!(result, Err(ValidationError::FutureDate(_))));
    }

    #[test]
    fn test_time_range_end_within_tolerance() {
        let now = Utc::now();
        // End is 30 minutes in the future — within the 1 hour tolerance
        let range = TimeRange {
            start: now - Duration::hours(1),
            end: now + Duration::minutes(30),
        };
        assert!(validate_time_range(&range).is_ok());
    }

    // ─── validate_settings ─────────────────────────────────────────────

    #[test]
    fn test_valid_settings_all_fields() {
        let settings = PartialSettings {
            locale: Some("th".to_string()),
            collection_interval_secs: Some(30),
            retention_days: Some(365),
            always_on_top: Some(true),
            click_through: Some(false),
            autostart: Some(true),
            notification_warning_pct: Some(75.0),
            notification_critical_pct: Some(90.0),
        };
        assert!(validate_settings(&settings).is_ok());
    }

    #[test]
    fn test_notification_thresholds_out_of_range_rejected() {
        for pct in [-1.0, 101.0] {
            let settings = PartialSettings {
                notification_warning_pct: Some(pct),
                ..Default::default()
            };
            assert!(
                validate_settings(&settings).is_err(),
                "{}% should be rejected",
                pct
            );
        }
    }

    #[test]
    fn test_warning_threshold_must_stay_below_critical() {
        let settings = PartialSettings {
            notification_warning_pct: Some(90.0),
            notification_critical_pct: Some(75.0),
            ..Default::default()
        };
        assert!(validate_settings(&settings).is_err());

        let ok = PartialSettings {
            notification_warning_pct: Some(50.0),
            notification_critical_pct: Some(80.0),
            ..Default::default()
        };
        assert!(validate_settings(&ok).is_ok());
    }

    #[test]
    fn test_valid_settings_english_locale() {
        let settings = PartialSettings {
            locale: Some("en".to_string()),
            ..Default::default()
        };
        assert!(validate_settings(&settings).is_ok());
    }

    #[test]
    fn test_valid_settings_none_fields() {
        let settings = PartialSettings::default();
        assert!(validate_settings(&settings).is_ok());
    }

    #[test]
    fn test_invalid_locale() {
        let settings = PartialSettings {
            locale: Some("fr".to_string()),
            ..Default::default()
        };
        let result = validate_settings(&settings);
        assert!(matches!(result, Err(ValidationError::InvalidLocale(_))));
    }

    #[test]
    fn test_interval_below_minimum() {
        let settings = PartialSettings {
            collection_interval_secs: Some(5),
            ..Default::default()
        };
        assert_eq!(
            validate_settings(&settings),
            Err(ValidationError::InvalidInterval(5, 10, 3600))
        );
    }

    #[test]
    fn test_interval_above_maximum() {
        let settings = PartialSettings {
            collection_interval_secs: Some(7200),
            ..Default::default()
        };
        assert_eq!(
            validate_settings(&settings),
            Err(ValidationError::InvalidInterval(7200, 10, 3600))
        );
    }

    #[test]
    fn test_interval_at_boundaries() {
        let min_settings = PartialSettings {
            collection_interval_secs: Some(10),
            ..Default::default()
        };
        assert!(validate_settings(&min_settings).is_ok());

        let max_settings = PartialSettings {
            collection_interval_secs: Some(3600),
            ..Default::default()
        };
        assert!(validate_settings(&max_settings).is_ok());
    }

    #[test]
    fn test_retention_days_below_minimum() {
        let settings = PartialSettings {
            retention_days: Some(0),
            ..Default::default()
        };
        assert_eq!(
            validate_settings(&settings),
            Err(ValidationError::InvalidRetentionDays(0, 1, 3650))
        );
    }

    #[test]
    fn test_retention_days_above_maximum() {
        let settings = PartialSettings {
            retention_days: Some(5000),
            ..Default::default()
        };
        assert_eq!(
            validate_settings(&settings),
            Err(ValidationError::InvalidRetentionDays(5000, 1, 3650))
        );
    }

    #[test]
    fn test_retention_days_at_boundaries() {
        let min_settings = PartialSettings {
            retention_days: Some(1),
            ..Default::default()
        };
        assert!(validate_settings(&min_settings).is_ok());

        let max_settings = PartialSettings {
            retention_days: Some(3650),
            ..Default::default()
        };
        assert!(validate_settings(&max_settings).is_ok());
    }

    // ─── validate_granularity ──────────────────────────────────────────

    #[test]
    fn test_all_granularities_valid() {
        assert!(validate_granularity(&Granularity::Hourly).is_ok());
        assert!(validate_granularity(&Granularity::Daily).is_ok());
        assert!(validate_granularity(&Granularity::Weekly).is_ok());
        assert!(validate_granularity(&Granularity::Monthly).is_ok());
    }

    // ─── validate_provider_id ──────────────────────────────────────────

    #[test]
    fn test_valid_provider_ids() {
        assert!(validate_provider_id("codex").is_ok());
        assert!(validate_provider_id("claude").is_ok());
        assert!(validate_provider_id("my-provider").is_ok());
        assert!(validate_provider_id("provider_v2").is_ok());
        assert!(validate_provider_id("Provider123").is_ok());
        assert!(validate_provider_id("a").is_ok());
    }

    #[test]
    fn test_empty_provider_id() {
        let result = validate_provider_id("");
        assert!(matches!(result, Err(ValidationError::InvalidProviderId(_))));
    }

    #[test]
    fn test_provider_id_with_spaces() {
        let result = validate_provider_id("my provider");
        assert!(matches!(result, Err(ValidationError::InvalidProviderId(_))));
    }

    #[test]
    fn test_provider_id_with_special_chars() {
        assert!(validate_provider_id("provider!").is_err());
        assert!(validate_provider_id("provider@name").is_err());
        assert!(validate_provider_id("provider/name").is_err());
        assert!(validate_provider_id("provider.name").is_err());
    }
}

#[cfg(test)]
mod prop_tests_validation {
    use super::*;
    use chrono::{Duration, Utc};
    use proptest::prelude::*;
    use proptest::test_runner::Config;

    // **Validates: Requirements 11.6, 11.7**
    //    // Property: Time ranges where start >= end must be rejected.
    proptest! {
        #![proptest_config(Config::with_cases(50))]
        #[test]
        fn start_gte_end_rejected(offset_secs in 0i64..=86400 * 400) {
            let now = Utc::now();
            let end = now - Duration::hours(2); // safely in the past
            let start = end + Duration::seconds(offset_secs); // start >= end

            let range = TimeRange { start, end };
            prop_assert!(validate_time_range(&range).is_err(),
                "Expected rejection when start >= end, but got Ok");
        }
    }

    // **Validates: Requirements 11.6, 11.7**
    //    // Property: Time ranges longer than 366 days must be rejected.
    proptest! {
        #![proptest_config(Config::with_cases(50))]
        #[test]
        fn range_over_366_days_rejected(extra_days in 1i64..=1000) {
            let now = Utc::now();
            let total_days = 366 + extra_days;
            let start = now - Duration::days(total_days);
            let end = now;

            let range = TimeRange { start, end };
            prop_assert!(validate_time_range(&range).is_err(),
                "Expected rejection for range > 366 days, but got Ok");
        }
    }

    // **Validates: Requirements 11.6, 11.7**
    //    // Property: Time ranges with end dates far in the future (> 1 hour) must be rejected.
    proptest! {
        #![proptest_config(Config::with_cases(50))]
        #[test]
        fn future_end_dates_rejected(extra_hours in 2i64..=8760) {
            let now = Utc::now();
            // end is more than 1 hour in the future
            let end = now + Duration::hours(extra_hours);
            let start = now - Duration::hours(1);

            let range = TimeRange { start, end };
            prop_assert!(validate_time_range(&range).is_err(),
                "Expected rejection for future end date, but got Ok");
        }
    }

    // **Validates: Requirements 11.6, 11.7**
    //    // Property: Valid ranges (start < end, duration <= 366 days, end not far in future) must be accepted.
    proptest! {
        #![proptest_config(Config::with_cases(50))]
        #[test]
        fn valid_ranges_accepted(duration_secs in 1i64..=(366 * 86400)) {
            let now = Utc::now();
            // End is in the past (safely within tolerance)
            let end = now - Duration::seconds(1);
            let start = end - Duration::seconds(duration_secs);

            let range = TimeRange { start, end };
            prop_assert!(validate_time_range(&range).is_ok(),
                "Expected valid range to be accepted, but got Err: {:?}",
                validate_time_range(&range));
        }
    }

    // **Validates: Requirements 11.6, 11.7**
    //    // Property: Collection intervals outside 10-3600 must be rejected; values within range must be accepted.
    proptest! {
        #![proptest_config(Config::with_cases(50))]
        #[test]
        fn interval_validation(interval in 0u32..=7200) {
            let settings = PartialSettings {
                collection_interval_secs: Some(interval),
                ..Default::default()
            };
            let result = validate_settings(&settings);

            if (10..=3600).contains(&interval) {
                prop_assert!(result.is_ok(),
                    "Expected interval {} to be accepted, but got Err: {:?}",
                    interval, result);
            } else {
                prop_assert!(result.is_err(),
                    "Expected interval {} to be rejected, but got Ok",
                    interval);
            }
        }
    }
}
