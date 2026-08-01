//! Quota observation-timestamp behaviour for the Codex adapter.
//!
//! A Codex percentage is only as current as the JSONL record that carried it.
//! These tests pin that: the timestamp must come from the record, the two
//! windows must age independently, and a quiet collection cycle must not make
//! an old reading look new.

use super::*;
use crate::freshness::{Freshness, QuotaTiming};
use std::fs;

/// A session log whose lines are given verbatim.
fn adapter_with_lines(dir: &std::path::Path, lines: &[String]) -> CodexAdapter {
    let day = dir.join("2026").join("07").join("30");
    fs::create_dir_all(&day).unwrap();
    fs::write(
        day.join("rollout-test.jsonl"),
        format!("{}\n", lines.join("\n")),
    )
    .unwrap();

    CodexAdapter::new(&CodexConfig {
        sessions_dir: dir.to_path_buf(),
        state_db_path: dir.join("missing.sqlite"),
        enabled: true,
    })
}

/// One `token_count` line stamped at `ts` carrying `rate_limits`.
fn line(ts: &str, rate_limits: &str) -> String {
    format!(
        r#"{{"timestamp":"{}","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"total_tokens":10}}}},"rate_limits":{}}}}}"#,
        ts, rate_limits
    )
}

fn at(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .with_timezone(&Utc)
}

/// Five-hour window only; weekly slot explicitly null.
const FAST_ONLY: &str = r#"{"limit_id":"codex","primary":{"used_percent":40.0,"window_minutes":300,"resets_at":1785922677},"secondary":null}"#;

#[test]
fn test_observation_time_comes_from_the_record_not_the_clock() {
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with_lines(dir.path(), &[line("2026-07-30T20:54:57.240Z", FAST_ONLY)]);
    adapter.collect(None).unwrap();

    let observed = adapter.quota_observed();
    assert_eq!(observed.fast_hours, Some(at("2026-07-30T20:54:57.240Z")));
    // The secondary slot was null, so the weekly window learned nothing
    assert_eq!(observed.weekly, None);
}

#[test]
fn test_each_window_keeps_its_own_observation_time() {
    // Codex stops publishing the five-hour window while the weekly limit
    // binds. The five-hour reading must keep its own age rather than inherit
    // the freshness of the line that only mentioned the weekly one.
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with_lines(
        dir.path(),
        &[
            line(
                "2026-07-30T10:00:00.000Z",
                r#"{"limit_id":"codex","primary":{"used_percent":40.0,"window_minutes":300,"resets_at":1785922677},"secondary":{"used_percent":10.0,"window_minutes":10080,"resets_at":1786193977}}"#,
            ),
            line(
                "2026-08-01T18:00:00.000Z",
                r#"{"limit_id":"codex","primary":{"used_percent":95.0,"window_minutes":10080,"resets_at":1786193977},"secondary":null}"#,
            ),
        ],
    );
    adapter.collect(None).unwrap();

    let observed = adapter.quota_observed();
    assert_eq!(
        observed.fast_hours,
        Some(at("2026-07-30T10:00:00.000Z")),
        "five-hour window must keep the older timestamp"
    );
    assert_eq!(
        observed.weekly,
        Some(at("2026-08-01T18:00:00.000Z")),
        "weekly window advanced with the newer line"
    );
}

#[test]
fn test_a_null_limit_family_does_not_refresh_a_valid_reading() {
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with_lines(
        dir.path(),
        &[
            line("2026-07-30T10:00:00.000Z", FAST_ONLY),
            line(
                "2026-08-01T18:00:00.000Z",
                r#"{"limit_id":"premium","primary":null,"secondary":null}"#,
            ),
        ],
    );
    adapter.collect(None).unwrap();

    assert_eq!(
        adapter.quota_observed().fast_hours,
        Some(at("2026-07-30T10:00:00.000Z"))
    );
}

#[test]
fn test_a_record_with_no_timestamp_yields_unknown_not_fresh() {
    // The collection path falls back to `Utc::now()` for a missing timestamp
    // when dating *usage events*. That fallback must not reach the quota
    // observation, or an undated record would read as just-written.
    let dir = tempfile::tempdir().unwrap();
    let undated = format!(
        r#"{{"type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"total_tokens":10}}}},"rate_limits":{}}}}}"#,
        FAST_ONLY
    );
    let adapter = adapter_with_lines(dir.path(), &[undated]);
    adapter.collect(None).unwrap();

    let observed = adapter.quota_observed();
    assert_eq!(observed.fast_hours, None, "no timestamp means no known age");

    let timing = QuotaTiming {
        observed_at: observed.fast_hours,
        resets_at: Some(at("2026-08-08T12:00:00Z")),
    };
    assert_eq!(
        timing.freshness(at("2026-08-01T12:00:00Z")),
        Freshness::Unknown
    );
}

#[test]
fn test_an_unparseable_timestamp_yields_unknown_not_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with_lines(dir.path(), &[line("not-a-timestamp", FAST_ONLY)]);
    adapter.collect(None).unwrap();

    assert_eq!(adapter.quota_observed().fast_hours, None);
}

#[test]
fn test_an_unparseable_timestamp_does_not_inherit_an_earlier_valid_age() {
    // The percentage is new but its age is genuinely unknown. Keeping the
    // previous record's timestamp would date this reading to a moment that
    // did not produce it — a quieter version of the same lie.
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with_lines(
        dir.path(),
        &[
            line("2026-07-30T10:00:00.000Z", FAST_ONLY),
            line(
                "garbage",
                r#"{"limit_id":"codex","primary":{"used_percent":88.0,"window_minutes":300,"resets_at":1785922677},"secondary":null}"#,
            ),
        ],
    );
    adapter.collect(None).unwrap();

    let summary = adapter.get_current_summary().unwrap();
    assert_eq!(
        summary.quota.and_then(|q| q.fast_hours_pct),
        Some(88.0),
        "the newer percentage is still adopted"
    );
    assert_eq!(
        adapter.quota_observed().fast_hours,
        None,
        "but its age is unknown, not the previous record's"
    );
}

#[test]
fn test_a_cycle_with_no_new_payload_leaves_the_age_alone() {
    // Collect Now on a quiet machine must not make an old reading look new.
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with_lines(dir.path(), &[line("2026-07-30T10:00:00.000Z", FAST_ONLY)]);
    adapter.collect(None).unwrap();
    let first = adapter.quota_observed();

    // Nothing is appended between these cycles
    adapter.collect(None).unwrap();
    adapter.collect(None).unwrap();

    assert_eq!(adapter.quota_observed(), first);
}

#[test]
fn test_priming_from_disk_keeps_the_record_timestamp() {
    // Priming runs at startup with no prior state. Stamping it with the launch
    // time would reset the age of every reading on every restart.
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with_lines(dir.path(), &[line("2026-07-30T10:00:00.000Z", FAST_ONLY)]);

    adapter.prime_rate_limits();

    assert_eq!(
        adapter.quota_observed().fast_hours,
        Some(at("2026-07-30T10:00:00.000Z"))
    );
}

#[test]
fn test_metadata_persisted_before_this_field_loads_as_unknown() {
    // Backward compatibility: an older snapshot carries percentages but no
    // observation times, and must read as unknown age rather than as fresh.
    let legacy = r#"{"fast_used_pct":40.0,"fast_resets_at":null,"weekly_used_pct":100.0,"weekly_resets_at":null}"#;
    let snapshot: RateLimitSnapshot = serde_json::from_str(legacy).unwrap();

    assert_eq!(snapshot.fast_used_pct, Some(40.0));
    assert_eq!(snapshot.fast_observed_at, None);
    assert_eq!(snapshot.weekly_observed_at, None);

    let timing = QuotaTiming {
        observed_at: snapshot.fast_observed_at,
        resets_at: None,
    };
    assert_eq!(
        timing.freshness(at("2026-08-01T12:00:00Z")),
        Freshness::Unknown,
        "old metadata must never be presented as fresh"
    );
}

#[test]
fn test_a_two_day_old_codex_reading_is_stale() {
    // The reported bug: Codex had not run for two days, so its last percentage
    // sat on disk and was displayed as though it were current.
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with_lines(
        dir.path(),
        &[line(
            "2026-07-30T12:00:00.000Z",
            r#"{"limit_id":"codex","primary":{"used_percent":100.0,"window_minutes":10080,"resets_at":1786193977},"secondary":null}"#,
        )],
    );
    adapter.collect(None).unwrap();

    let now = at("2026-08-01T12:00:00Z");
    let timing = QuotaTiming {
        observed_at: adapter.quota_observed().weekly,
        // Reset is well in the future, so this is about age alone
        resets_at: Some(at("2026-08-08T12:00:00Z")),
    };

    assert_eq!(timing.freshness(now), Freshness::Stale);
    assert_eq!(timing.age_seconds(now), Some(2 * 86_400));
}

#[test]
fn test_a_reading_whose_window_already_rolled_over_is_expired() {
    let dir = tempfile::tempdir().unwrap();
    let adapter = adapter_with_lines(dir.path(), &[line("2026-08-01T11:59:00.000Z", FAST_ONLY)]);
    adapter.collect(None).unwrap();

    let now = at("2026-08-01T12:00:00Z");
    let timing = QuotaTiming {
        observed_at: adapter.quota_observed().fast_hours,
        resets_at: Some(at("2026-08-01T11:59:30Z")),
    };

    // Written a minute ago, but it describes a window that has since ended
    assert_eq!(timing.freshness(now), Freshness::Expired);
}

#[test]
fn test_new_metadata_is_readable_by_the_old_contract() {
    // Rollback safety: an older binary deserialises this branch's snapshot
    // with serde ignoring the two unknown keys. Nothing about the extra
    // fields can make the old shape unreadable, so reverting the PR needs no
    // database repair.
    #[derive(serde::Deserialize)]
    struct LegacySnapshot {
        fast_used_pct: Option<f64>,
        weekly_used_pct: Option<f64>,
        #[serde(default)]
        weekly_resets_at: Option<DateTime<Utc>>,
    }

    let new_snapshot = RateLimitSnapshot {
        fast_used_pct: Some(40.0),
        fast_resets_at: None,
        weekly_used_pct: Some(100.0),
        weekly_resets_at: Some(at("2026-08-08T12:00:00Z")),
        fast_observed_at: Some(at("2026-08-01T11:00:00Z")),
        weekly_observed_at: Some(at("2026-07-30T12:00:00Z")),
    };
    let json = serde_json::to_string(&new_snapshot).unwrap();

    let legacy: LegacySnapshot = serde_json::from_str(&json).expect("old shape still parses");
    assert_eq!(legacy.fast_used_pct, Some(40.0));
    assert_eq!(legacy.weekly_used_pct, Some(100.0));
    assert_eq!(legacy.weekly_resets_at, Some(at("2026-08-08T12:00:00Z")));
}

#[test]
fn test_a_round_trip_through_persistence_keeps_the_observed_times() {
    let snapshot = RateLimitSnapshot {
        fast_used_pct: Some(40.0),
        fast_resets_at: None,
        weekly_used_pct: Some(100.0),
        weekly_resets_at: None,
        fast_observed_at: Some(at("2026-08-01T11:00:00Z")),
        weekly_observed_at: Some(at("2026-07-30T12:00:00Z")),
    };

    let restored: RateLimitSnapshot =
        serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();

    assert_eq!(restored.fast_observed_at, snapshot.fast_observed_at);
    assert_eq!(restored.weekly_observed_at, snapshot.weekly_observed_at);
}
