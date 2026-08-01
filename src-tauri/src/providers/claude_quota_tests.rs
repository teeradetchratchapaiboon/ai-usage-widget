//! Quota observation-timestamp behaviour for the Claude adapter.
//!
//! Claude stamps every sample in `plan-usage-history.json` with `t`, so unlike
//! its reset times — which are reconstructed from drops — the age of a reading
//! is stated outright. These tests pin that the stated value is what is used,
//! and that file metadata and collection time never leak in.

use super::*;
use crate::freshness::{Freshness, QuotaTiming};
use std::fs;

fn at(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .with_timezone(&Utc)
}

/// Milliseconds since the epoch for an RFC 3339 instant.
fn ms(rfc3339: &str) -> i64 {
    at(rfc3339).timestamp_millis()
}

/// Write a v2 history holding the given `(t_ms, fh, sd)` samples.
fn adapter_with(
    dir: &std::path::Path,
    samples: &[(i64, Option<f64>, Option<f64>)],
) -> ClaudeAdapter {
    let entries: Vec<String> = samples
        .iter()
        .map(|(t, fh, sd)| {
            let fh = fh.map(|v| v.to_string()).unwrap_or_else(|| "null".into());
            let sd = sd.map(|v| v.to_string()).unwrap_or_else(|| "null".into());
            format!(r#"{{"t":{},"org":"o","u":{{"fh":{},"sd":{}}}}}"#, t, fh, sd)
        })
        .collect();

    fs::write(
        dir.join("plan-usage-history.json"),
        format!(r#"{{"version":2,"samples":[{}]}}"#, entries.join(",")),
    )
    .unwrap();

    ClaudeAdapter::new(dir.to_path_buf())
}

#[test]
fn test_observation_time_is_the_latest_sample_timestamp() {
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with(
        dir.path(),
        &[
            (ms("2026-08-01T09:00:00Z"), Some(10.0), Some(20.0)),
            (ms("2026-08-01T11:45:00Z"), Some(30.0), Some(40.0)),
        ],
    );

    let observed = adapter.quota_observed();
    assert_eq!(observed.fast_hours, Some(at("2026-08-01T11:45:00Z")));
    assert_eq!(observed.weekly, Some(at("2026-08-01T11:45:00Z")));
}

#[test]
fn test_the_summary_reports_the_same_observation_as_the_trait() {
    // The two must not drift: the widget reads one and the fallback the other.
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with(
        dir.path(),
        &[(ms("2026-08-01T11:45:00Z"), Some(30.0), Some(40.0))],
    );

    let summary = adapter.get_current_summary().unwrap();
    assert_eq!(summary.quota_observed, adapter.quota_observed());
    assert_eq!(
        summary.quota_observed.fast_hours,
        Some(at("2026-08-01T11:45:00Z"))
    );
}

#[test]
fn test_a_window_the_sample_does_not_report_gets_no_timestamp() {
    // There is no reading to be fresh or stale, so claiming an age for it
    // would be inventing one.
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with(
        dir.path(),
        &[(ms("2026-08-01T11:45:00Z"), None, Some(40.0))],
    );

    let observed = adapter.quota_observed();
    assert_eq!(observed.fast_hours, None);
    assert_eq!(observed.weekly, Some(at("2026-08-01T11:45:00Z")));
}

#[test]
fn test_file_metadata_is_not_used_as_the_observation_time() {
    // The file is written now, but its newest sample is two days old. Falling
    // back to the mtime would report that reading as current.
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with(
        dir.path(),
        &[(ms("2026-07-30T12:00:00Z"), Some(90.0), Some(95.0))],
    );

    let now = at("2026-08-01T12:00:00Z");
    let timing = QuotaTiming {
        observed_at: adapter.quota_observed().weekly,
        resets_at: Some(at("2026-08-08T12:00:00Z")),
    };

    assert_eq!(timing.age_seconds(now), Some(2 * 86_400));
    assert_eq!(timing.freshness(now), Freshness::Stale);
}

#[test]
fn test_missing_history_yields_unknown_rather_than_now() {
    let dir = tempfile::tempdir().unwrap();
    let adapter = ClaudeAdapter::new(dir.path().to_path_buf());

    let observed = adapter.quota_observed();
    assert!(observed.is_empty());

    let timing = QuotaTiming {
        observed_at: observed.weekly,
        resets_at: None,
    };
    assert_eq!(
        timing.freshness(at("2026-08-01T12:00:00Z")),
        Freshness::Unknown
    );
}

#[test]
fn test_estimated_reset_behaviour_is_unchanged_by_observation_tracking() {
    // Reset confidence and reading freshness are separate axes; adding the
    // second must not disturb the first.
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with(
        dir.path(),
        &[
            (ms("2026-08-01T09:00:00Z"), Some(40.0), Some(50.0)),
            (ms("2026-08-01T10:00:00Z"), Some(3.0), Some(51.0)),
            (ms("2026-08-01T11:45:00Z"), Some(22.0), Some(52.0)),
        ],
    );

    assert!(
        adapter.quota_resets().estimated,
        "Claude reset times remain reconstructed"
    );
}
