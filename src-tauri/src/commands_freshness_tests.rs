//! The freshness half of the provider-status wire contract.
//!
//! The mapping is the last place a correct backend can still lie to the UI: a
//! hardcoded `"fresh"` here would pass every adapter test in the suite while
//! presenting a two-day-old percentage as current.

use super::*;
use crate::provider::{QuotaObservations, QuotaResets};
use crate::types::QuotaUsage;
use chrono::Duration;

fn at(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .with_timezone(&Utc)
}

/// A fixed clock, so nothing below races the wall clock.
fn now() -> DateTime<Utc> {
    at("2026-08-01T12:00:00Z")
}

fn summary_with(
    observed: QuotaObservations,
    resets: QuotaResets,
    quota: Option<QuotaUsage>,
) -> ProviderSummary {
    ProviderSummary {
        provider_id: "codex".to_string(),
        display_name: "Codex Desktop".to_string(),
        is_available: true,
        current_model: None,
        tokens_today: Some(31_000),
        quota,
        context_window: None,
        last_activity: Some(at("2026-08-01T11:58:00Z")),
        quota_resets: resets,
        quota_observed: observed,
    }
}

fn future_resets() -> QuotaResets {
    QuotaResets {
        fast_hours: Some(at("2026-08-01T15:00:00Z")),
        weekly: Some(at("2026-08-08T15:00:00Z")),
        estimated: false,
    }
}

#[test]
fn test_observation_times_reach_the_wire_as_rfc3339() {
    let observed = QuotaObservations {
        fast_hours: Some(at("2026-08-01T11:55:00Z")),
        weekly: Some(at("2026-07-30T12:00:00Z")),
    };
    let response =
        provider_status_response_at(summary_with(observed, future_resets(), None), now());

    assert_eq!(
        response.quota_fast_observed_at.as_deref(),
        Some("2026-08-01T11:55:00+00:00")
    );
    assert_eq!(
        response.quota_weekly_observed_at.as_deref(),
        Some("2026-07-30T12:00:00+00:00")
    );
}

#[test]
fn test_each_window_is_classified_on_its_own_reading() {
    // The exact shape of the reported bug: a five-hour window written minutes
    // ago beside a weekly window two days old. One badge for both would
    // launder the stale one.
    let observed = QuotaObservations {
        fast_hours: Some(at("2026-08-01T11:55:00Z")),
        weekly: Some(at("2026-07-30T12:00:00Z")),
    };
    let response =
        provider_status_response_at(summary_with(observed, future_resets(), None), now());

    assert_eq!(response.quota_fast_freshness, "fresh");
    assert_eq!(response.quota_weekly_freshness, "stale");
    assert_eq!(response.quota_fast_age_secs, Some(5 * 60));
    assert_eq!(response.quota_weekly_age_secs, Some(2 * 86_400));
}

#[test]
fn test_aging_sits_between_fresh_and_stale() {
    let observed = QuotaObservations {
        fast_hours: Some(now() - Duration::hours(2)),
        weekly: Some(now() - Duration::minutes(20)),
    };
    let response =
        provider_status_response_at(summary_with(observed, future_resets(), None), now());

    assert_eq!(response.quota_fast_freshness, "aging");
    assert_eq!(response.quota_weekly_freshness, "aging");
}

#[test]
fn test_a_past_reset_is_expired_even_with_a_recent_observation() {
    let observed = QuotaObservations {
        fast_hours: Some(now() - Duration::minutes(1)),
        weekly: Some(now() - Duration::minutes(1)),
    };
    let resets = QuotaResets {
        fast_hours: Some(now() - Duration::seconds(1)),
        weekly: Some(now() + Duration::days(7)),
        estimated: false,
    };
    let response = provider_status_response_at(summary_with(observed, resets, None), now());

    assert_eq!(response.quota_fast_freshness, "expired");
    assert_eq!(response.quota_weekly_freshness, "fresh");
}

#[test]
fn test_absent_observations_map_to_unknown_not_fresh() {
    let response = provider_status_response_at(
        summary_with(QuotaObservations::default(), future_resets(), None),
        now(),
    );

    assert_eq!(response.quota_fast_freshness, "unknown");
    assert_eq!(response.quota_weekly_freshness, "unknown");
    assert_eq!(response.quota_fast_observed_at, None);
    assert_eq!(response.quota_fast_age_secs, None);
}

#[test]
fn test_freshness_is_independent_of_reset_confidence() {
    // Claude: reconstructed reset, but a percentage written a minute ago.
    let observed = QuotaObservations {
        fast_hours: Some(now() - Duration::minutes(1)),
        weekly: Some(now() - Duration::minutes(1)),
    };
    let resets = QuotaResets {
        fast_hours: Some(now() + Duration::hours(3)),
        weekly: Some(now() + Duration::days(6)),
        estimated: true,
    };
    let response = provider_status_response_at(summary_with(observed, resets, None), now());

    assert!(response.quota_resets_estimated);
    assert_eq!(response.quota_fast_freshness, "fresh");
}

#[test]
fn test_percentages_still_reach_the_wire_untouched() {
    // Freshness annotates a reading; it must never rewrite or invent one.
    let quota = QuotaUsage {
        fast_hours_pct: Some(40.0),
        standard_pct: Some(100.0),
        excess_pct: None,
        daily_tokens: None,
    };
    let response = provider_status_response_at(
        summary_with(QuotaObservations::default(), future_resets(), Some(quota)),
        now(),
    );

    assert_eq!(response.quota_fast_pct, Some(40.0));
    assert_eq!(response.quota_standard_pct, Some(100.0));
    assert_eq!(response.quota_excess_pct, None);
}

#[test]
fn test_activity_time_is_reported_as_activity_not_as_collection() {
    // The field used to be called `last_collection` while carrying provider
    // activity, which invited reading it as the age of the quota.
    let response = provider_status_response_at(
        summary_with(QuotaObservations::default(), future_resets(), None),
        now(),
    );

    assert_eq!(
        response.last_activity.as_deref(),
        Some("2026-08-01T11:58:00+00:00")
    );
}

#[test]
fn test_tokens_today_is_not_republished_as_an_event_count() {
    // `events_collected` used to be populated from `tokens_today`; 31,000
    // tokens is not 31,000 events, and no field should claim it is.
    let response = provider_status_response_at(
        summary_with(QuotaObservations::default(), future_resets(), None),
        now(),
    );

    assert_eq!(response.tokens_today, Some(31_000));

    let json = serde_json::to_value(&response).unwrap();
    assert!(
        json.get("events_collected").is_none(),
        "the misleading field must be gone from the wire, not merely unused"
    );
    assert!(json.get("last_collection").is_none());
}

#[test]
fn test_the_wire_form_uses_the_documented_lowercase_states() {
    let json = serde_json::to_value(provider_status_response_at(
        summary_with(QuotaObservations::default(), future_resets(), None),
        now(),
    ))
    .unwrap();

    assert_eq!(json["quota_fast_freshness"], "unknown");
    for key in [
        "quota_fast_observed_at",
        "quota_weekly_observed_at",
        "quota_fast_freshness",
        "quota_weekly_freshness",
        "quota_fast_age_secs",
        "quota_weekly_age_secs",
    ] {
        assert!(json.get(key).is_some(), "missing wire field {key}");
    }
}
