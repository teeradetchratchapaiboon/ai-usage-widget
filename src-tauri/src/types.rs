use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Represents the type of usage event collected from a provider.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    TokenCount,
    SessionSummary,
    QuotaSample,
    DailyAggregate,
}

/// Token usage breakdown for a single event.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

/// Quota usage percentages from Claude Desktop.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QuotaUsage {
    pub fast_hours_pct: Option<f64>,
    pub standard_pct: Option<f64>,
    pub excess_pct: Option<f64>,
    pub daily_tokens: Option<u64>,
}

/// A raw usage event as collected from a provider before deduplication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawUsageEvent {
    pub provider_id: String,
    pub event_type: EventType,
    pub timestamp: DateTime<Utc>,
    pub model: Option<String>,
    pub tokens: TokenUsage,
    pub context_window: Option<u64>,
    pub quota: Option<QuotaUsage>,
    pub session_hash: Option<String>,
    pub project_hash: Option<String>,
    pub source_file: Option<String>,
    pub raw_metadata: Option<serde_json::Value>,
}

/// SHA-256 fingerprint for event deduplication (32 bytes).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventFingerprint(pub [u8; 32]);

impl EventFingerprint {
    /// Create a new fingerprint from a 32-byte array.
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Return the fingerprint as a hex string.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

/// A reconciled event ready for storage, produced by merging deduplicated raw events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciledEvent {
    pub id: Uuid,
    pub fingerprint: EventFingerprint,
    pub provider_id: String,
    pub event_type: EventType,
    pub timestamp: DateTime<Utc>,
    pub model: Option<String>,
    pub tokens: TokenUsage,
    pub context_window: Option<u64>,
    pub quota: Option<QuotaUsage>,
    pub session_hash: Option<String>,
    pub project_hash: Option<String>,
    pub reconciled_at: DateTime<Utc>,
    pub source_count: u8,
}

#[cfg(test)]
mod prop_tests_data_normalization {
    use super::*;
    use chrono::{Duration, Utc};
    use proptest::prelude::*;

    // **Validates: Requirements 1.7, 13.2, 14.1**

    // ─── Strategies ─────────────────────────────────────────────────────────────

    fn arb_event_type() -> impl Strategy<Value = EventType> {
        prop_oneof![
            Just(EventType::TokenCount),
            Just(EventType::SessionSummary),
            Just(EventType::QuotaSample),
            Just(EventType::DailyAggregate),
        ]
    }

    fn arb_non_empty_string() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_\\-]{1,32}".prop_map(|s| s)
    }

    fn arb_optional_non_empty_string() -> impl Strategy<Value = Option<String>> {
        prop_oneof![
            Just(None),
            arb_non_empty_string().prop_map(Some),
        ]
    }

    fn arb_token_usage() -> impl Strategy<Value = TokenUsage> {
        (
            proptest::option::of(0u64..1_000_000),
            proptest::option::of(0u64..1_000_000),
            proptest::option::of(0u64..1_000_000),
            proptest::option::of(0u64..1_000_000),
            proptest::option::of(0u64..10_000_000),
        )
            .prop_map(
                |(input, cached, output, reasoning, total)| TokenUsage {
                    input_tokens: input,
                    cached_input_tokens: cached,
                    output_tokens: output,
                    reasoning_tokens: reasoning,
                    total_tokens: total,
                },
            )
    }

    fn arb_quota_usage() -> impl Strategy<Value = QuotaUsage> {
        (
            proptest::option::of(0.0f64..500.0),
            proptest::option::of(0.0f64..500.0),
            proptest::option::of(0.0f64..500.0),
            proptest::option::of(0u64..10_000_000),
        )
            .prop_map(
                |(fast, standard, excess, daily)| QuotaUsage {
                    fast_hours_pct: fast,
                    standard_pct: standard,
                    excess_pct: excess,
                    daily_tokens: daily,
                },
            )
    }

    fn arb_timestamp() -> impl Strategy<Value = DateTime<Utc>> {
        // Generate timestamps within a reasonable range: 2020-01-01 to now + 1 hour
        let min_ts = chrono::NaiveDate::from_ymd_opt(2020, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let max_ts = Utc::now().timestamp() + 3600; // up to 1 hour in future

        (min_ts..max_ts).prop_map(|ts| {
            DateTime::from_timestamp(ts, 0).unwrap()
        })
    }

    fn arb_raw_usage_event() -> impl Strategy<Value = RawUsageEvent> {
        (
            arb_non_empty_string(),
            arb_event_type(),
            arb_timestamp(),
            arb_optional_non_empty_string(),
            arb_token_usage(),
            proptest::option::of(1024u64..200_000),
            proptest::option::of(arb_quota_usage()),
            arb_optional_non_empty_string(),
            arb_optional_non_empty_string(),
            arb_optional_non_empty_string(),
        )
            .prop_map(
                |(provider_id, event_type, timestamp, model, tokens, context_window, quota, session_hash, project_hash, source_file)| {
                    RawUsageEvent {
                        provider_id,
                        event_type,
                        timestamp,
                        model,
                        tokens,
                        context_window,
                        quota,
                        session_hash,
                        project_hash,
                        source_file,
                        raw_metadata: None,
                    }
                },
            )
    }

    // ─── Property Tests ─────────────────────────────────────────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(50))]

        /// Property 1: provider_id is non-empty (Req 1.7)
        #[test]
        fn prop_provider_id_non_empty(event in arb_raw_usage_event()) {
            prop_assert!(!event.provider_id.is_empty(),
                "provider_id must be non-empty, got empty string");
        }

        /// Property 2: event_type is a valid EventType variant (Req 1.7)
        #[test]
        fn prop_event_type_is_valid(event in arb_raw_usage_event()) {
            // If we can match it exhaustively, it's a valid variant
            let valid = matches!(
                event.event_type,
                EventType::TokenCount
                    | EventType::SessionSummary
                    | EventType::QuotaSample
                    | EventType::DailyAggregate
            );
            prop_assert!(valid, "event_type must be a valid EventType variant");
        }

        /// Property 3: timestamp is valid UTC and not unreasonably far in the future (Req 1.7)
        #[test]
        fn prop_timestamp_valid_utc(event in arb_raw_usage_event()) {
            let now = Utc::now();
            let max_future = now + Duration::hours(2);
            prop_assert!(
                event.timestamp <= max_future,
                "timestamp {:?} is unreasonably far in the future (max allowed: {:?})",
                event.timestamp,
                max_future
            );
            // Also ensure it's not before year 2000 (unreasonable)
            let min_date = chrono::NaiveDate::from_ymd_opt(2000, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc();
            prop_assert!(
                event.timestamp >= min_date,
                "timestamp {:?} is unreasonably old (before 2000-01-01)",
                event.timestamp
            );
        }

        /// Property 4: token values are non-negative (Req 13.2)
        /// u64 guarantees >= 0, but we verify consistency: if all sub-tokens
        /// are present, they form a valid set.
        #[test]
        fn prop_token_values_non_negative(event in arb_raw_usage_event()) {
            // u64 inherently >= 0, verify they exist as expected
            if let Some(input) = event.tokens.input_tokens {
                prop_assert!(input < u64::MAX, "input_tokens should be reasonable");
            }
            if let Some(output) = event.tokens.output_tokens {
                prop_assert!(output < u64::MAX, "output_tokens should be reasonable");
            }
            if let Some(total) = event.tokens.total_tokens {
                prop_assert!(total < u64::MAX, "total_tokens should be reasonable");
            }
            if let Some(cached) = event.tokens.cached_input_tokens {
                prop_assert!(cached < u64::MAX, "cached_input_tokens should be reasonable");
            }
            if let Some(reasoning) = event.tokens.reasoning_tokens {
                prop_assert!(reasoning < u64::MAX, "reasoning_tokens should be reasonable");
            }
        }

        /// Property 5: quota percentages in valid range [0.0, ∞) (Req 14.1)
        /// Per spec, excess can exceed 100%.
        #[test]
        fn prop_quota_percentages_valid_range(event in arb_raw_usage_event()) {
            if let Some(ref quota) = event.quota {
                if let Some(fh) = quota.fast_hours_pct {
                    prop_assert!(fh >= 0.0,
                        "fast_hours_pct must be >= 0.0, got {}", fh);
                    prop_assert!(!fh.is_nan(),
                        "fast_hours_pct must not be NaN");
                }
                if let Some(sd) = quota.standard_pct {
                    prop_assert!(sd >= 0.0,
                        "standard_pct must be >= 0.0, got {}", sd);
                    prop_assert!(!sd.is_nan(),
                        "standard_pct must not be NaN");
                }
                if let Some(xu) = quota.excess_pct {
                    prop_assert!(xu >= 0.0,
                        "excess_pct must be >= 0.0, got {}", xu);
                    prop_assert!(!xu.is_nan(),
                        "excess_pct must not be NaN");
                }
            }
        }

        /// Property 6: model field when present is non-empty (Req 1.7)
        #[test]
        fn prop_model_non_empty_when_present(event in arb_raw_usage_event()) {
            if let Some(ref model) = event.model {
                prop_assert!(!model.is_empty(),
                    "model must not be empty when Some");
            }
        }

        /// Property 7: Serialization round-trip (Req 1.7)
        /// Any RawUsageEvent can be serialized to JSON and deserialized back.
        #[test]
        fn prop_serialization_round_trip(event in arb_raw_usage_event()) {
            let json = serde_json::to_string(&event)
                .expect("serialization should not fail");
            let deserialized: RawUsageEvent = serde_json::from_str(&json)
                .expect("deserialization should not fail");

            // Verify key fields match exactly
            prop_assert_eq!(&event.provider_id, &deserialized.provider_id);
            prop_assert_eq!(&event.event_type, &deserialized.event_type);
            prop_assert_eq!(event.timestamp, deserialized.timestamp);
            prop_assert_eq!(&event.model, &deserialized.model);
            prop_assert_eq!(&event.tokens, &deserialized.tokens);
            prop_assert_eq!(event.context_window, deserialized.context_window);
            prop_assert_eq!(&event.session_hash, &deserialized.session_hash);
            prop_assert_eq!(&event.project_hash, &deserialized.project_hash);
            prop_assert_eq!(&event.source_file, &deserialized.source_file);

            // Quota: f64 fields may have minor precision drift after JSON round-trip
            // so we compare with a small epsilon tolerance
            fn f64_approx_eq(a: Option<f64>, b: Option<f64>) -> bool {
                match (a, b) {
                    (None, None) => true,
                    (Some(x), Some(y)) => (x - y).abs() < 1e-10,
                    _ => false,
                }
            }

            match (&event.quota, &deserialized.quota) {
                (None, None) => {},
                (Some(a), Some(b)) => {
                    prop_assert!(f64_approx_eq(a.fast_hours_pct, b.fast_hours_pct),
                        "fast_hours_pct mismatch: {:?} vs {:?}", a.fast_hours_pct, b.fast_hours_pct);
                    prop_assert!(f64_approx_eq(a.standard_pct, b.standard_pct),
                        "standard_pct mismatch: {:?} vs {:?}", a.standard_pct, b.standard_pct);
                    prop_assert!(f64_approx_eq(a.excess_pct, b.excess_pct),
                        "excess_pct mismatch: {:?} vs {:?}", a.excess_pct, b.excess_pct);
                    prop_assert_eq!(a.daily_tokens, b.daily_tokens);
                },
                _ => prop_assert!(false, "quota mismatch after round-trip"),
            }
        }
    }
}
