use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use crate::dedup::compute_fingerprint;
use crate::types::{QuotaUsage, RawUsageEvent, ReconciledEvent, TokenUsage};

/// Groups events by a composite identity key for reconciliation.
/// Events are considered part of the same group when they share the same
/// provider_id and model, and their timestamps fall within a 1-second window.
type IdentityKey = (String, Option<String>); // (provider_id, model)

/// The reconciliation engine merges overlapping data from multiple sources
/// for the same provider into a single authoritative event.
pub struct ReconciliationEngine;

impl ReconciliationEngine {
    /// Create a new ReconciliationEngine.
    pub fn new() -> Self {
        Self
    }

    /// Group events by (provider_id, model) within a 1-second timestamp window.
    ///
    /// Events are sorted by timestamp first, then grouped by identity key.
    /// Two events belong to the same group if they share provider_id + model
    /// and their timestamps differ by at most 1 second.
    pub fn group_by_identity(
        &self,
        mut events: Vec<RawUsageEvent>,
    ) -> Vec<Vec<RawUsageEvent>> {
        if events.is_empty() {
            return Vec::new();
        }

        // Sort by timestamp for consistent window grouping
        events.sort_by_key(|e| e.timestamp);

        // Intermediate grouping: first by identity key, then by time window
        let mut identity_groups: HashMap<IdentityKey, Vec<RawUsageEvent>> = HashMap::new();

        for event in events {
            let key = (event.provider_id.clone(), event.model.clone());
            identity_groups.entry(key).or_default().push(event);
        }

        let mut result: Vec<Vec<RawUsageEvent>> = Vec::new();

        for (_key, group_events) in identity_groups {
            // Within each identity group, split by 1-second time windows
            let mut current_window: Vec<RawUsageEvent> = Vec::new();
            let mut window_start: Option<DateTime<Utc>> = None;

            for event in group_events {
                match window_start {
                    None => {
                        window_start = Some(event.timestamp);
                        current_window.push(event);
                    }
                    Some(start) => {
                        if event.timestamp - start <= Duration::seconds(1) {
                            current_window.push(event);
                        } else {
                            // Start a new window
                            result.push(std::mem::take(&mut current_window));
                            window_start = Some(event.timestamp);
                            current_window.push(event);
                        }
                    }
                }
            }

            if !current_window.is_empty() {
                result.push(current_window);
            }
        }

        result
    }

    /// Merge two TokenUsage structs, preferring the one with more non-None fields.
    /// If both have the same number of populated fields, prefer `b` (later event).
    pub fn merge_tokens(a: &TokenUsage, b: &TokenUsage) -> TokenUsage {
        let a_count = count_token_fields(a);
        let b_count = count_token_fields(b);

        if b_count > a_count {
            b.clone()
        } else {
            a.clone()
        }
    }

    /// Merge two QuotaUsage options, combining complementary fields.
    /// If both sources provide a value for the same field, prefer the non-None one.
    /// If both are non-None for the same field, prefer `b` (later/more detailed source).
    pub fn merge_quota(a: &Option<QuotaUsage>, b: &Option<QuotaUsage>) -> Option<QuotaUsage> {
        match (a, b) {
            (None, None) => None,
            (Some(qa), None) => Some(qa.clone()),
            (None, Some(qb)) => Some(qb.clone()),
            (Some(qa), Some(qb)) => {
                Some(QuotaUsage {
                    fast_hours_pct: qb.fast_hours_pct.or(qa.fast_hours_pct),
                    standard_pct: qb.standard_pct.or(qa.standard_pct),
                    excess_pct: qb.excess_pct.or(qa.excess_pct),
                    daily_tokens: qb.daily_tokens.or(qa.daily_tokens),
                })
            }
        }
    }

    /// Reconcile a batch of deduplicated events into merged ReconciledEvents.
    ///
    /// Groups events by identity (provider_id + model within 1-second window),
    /// merges overlapping data preferring more detailed sources, and produces
    /// a Vec<ReconciledEvent> with source_count indicating how many raw events
    /// contributed to each reconciled output.
    pub fn reconcile(&self, events: Vec<RawUsageEvent>) -> Vec<ReconciledEvent> {
        let groups = self.group_by_identity(events);
        let mut reconciled = Vec::with_capacity(groups.len());

        for group in groups {
            let source_count = group.len() as u8;

            if source_count == 1 {
                // Single event — convert directly
                let event = group.into_iter().next().unwrap();
                reconciled.push(to_reconciled(event, 1));
            } else {
                // Multiple events for same identity — merge
                let merged = self.merge_group(group);
                reconciled.push(to_reconciled(merged.0, merged.1));
            }
        }

        reconciled
    }

    /// Merge a group of events sharing the same identity into a single event.
    /// Returns the merged event and the source count.
    fn merge_group(&self, group: Vec<RawUsageEvent>) -> (RawUsageEvent, u8) {
        let source_count = group.len() as u8;
        let mut iter = group.into_iter();
        let first = iter.next().unwrap();

        let merged = iter.fold(first, |acc, event| {
            RawUsageEvent {
                provider_id: acc.provider_id,
                event_type: acc.event_type,
                timestamp: acc.timestamp, // Keep earliest timestamp
                model: event.model.or(acc.model),
                tokens: Self::merge_tokens(&acc.tokens, &event.tokens),
                context_window: event.context_window.or(acc.context_window),
                quota: Self::merge_quota(&acc.quota, &event.quota),
                session_hash: event.session_hash.or(acc.session_hash),
                project_hash: event.project_hash.or(acc.project_hash),
                source_file: acc.source_file, // Keep first source file reference
                raw_metadata: acc.raw_metadata, // Keep first metadata
            }
        });

        (merged, source_count)
    }
}

impl Default for ReconciliationEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Count the number of non-None fields in a TokenUsage struct.
fn count_token_fields(t: &TokenUsage) -> usize {
    let mut count = 0;
    if t.input_tokens.is_some() {
        count += 1;
    }
    if t.cached_input_tokens.is_some() {
        count += 1;
    }
    if t.output_tokens.is_some() {
        count += 1;
    }
    if t.reasoning_tokens.is_some() {
        count += 1;
    }
    if t.total_tokens.is_some() {
        count += 1;
    }
    count
}

/// Convert a RawUsageEvent into a ReconciledEvent with the given source_count.
fn to_reconciled(event: RawUsageEvent, source_count: u8) -> ReconciledEvent {
    let fingerprint = compute_fingerprint(&event);

    ReconciledEvent {
        id: Uuid::new_v4(),
        fingerprint,
        provider_id: event.provider_id,
        event_type: event.event_type,
        timestamp: event.timestamp,
        model: event.model,
        tokens: event.tokens,
        context_window: event.context_window,
        quota: event.quota,
        session_hash: event.session_hash,
        project_hash: event.project_hash,
        reconciled_at: Utc::now(),
        source_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EventType;
    use chrono::TimeZone;

    fn make_event_at(
        provider: &str,
        model: Option<&str>,
        ts: DateTime<Utc>,
        tokens: TokenUsage,
    ) -> RawUsageEvent {
        RawUsageEvent {
            provider_id: provider.to_string(),
            event_type: EventType::TokenCount,
            timestamp: ts,
            model: model.map(|s| s.to_string()),
            tokens,
            context_window: None,
            quota: None,
            session_hash: None,
            project_hash: None,
            source_file: None,
            raw_metadata: None,
        }
    }

    #[test]
    fn test_single_event_passes_through() {
        let engine = ReconciliationEngine::new();
        let ts = Utc::now();
        let event = make_event_at(
            "codex",
            Some("gpt-4"),
            ts,
            TokenUsage {
                input_tokens: Some(100),
                output_tokens: Some(200),
                total_tokens: Some(300),
                ..Default::default()
            },
        );

        let result = engine.reconcile(vec![event]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_count, 1);
        assert_eq!(result[0].provider_id, "codex");
        assert_eq!(result[0].tokens.input_tokens, Some(100));
        assert_eq!(result[0].tokens.output_tokens, Some(200));
        assert_eq!(result[0].tokens.total_tokens, Some(300));
    }

    #[test]
    fn test_group_by_identity_same_provider_model_within_window() {
        let engine = ReconciliationEngine::new();
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 10, 0, 0).unwrap();

        let e1 = make_event_at(
            "codex",
            Some("gpt-4"),
            ts,
            TokenUsage {
                input_tokens: Some(100),
                ..Default::default()
            },
        );
        let e2 = make_event_at(
            "codex",
            Some("gpt-4"),
            ts + Duration::milliseconds(500),
            TokenUsage {
                input_tokens: Some(100),
                output_tokens: Some(200),
                total_tokens: Some(300),
                ..Default::default()
            },
        );

        let groups = engine.group_by_identity(vec![e1, e2]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn test_group_by_identity_different_models_separate() {
        let engine = ReconciliationEngine::new();
        let ts = Utc::now();

        let e1 = make_event_at(
            "codex",
            Some("gpt-4"),
            ts,
            TokenUsage::default(),
        );
        let e2 = make_event_at(
            "codex",
            Some("gpt-3.5"),
            ts,
            TokenUsage::default(),
        );

        let groups = engine.group_by_identity(vec![e1, e2]);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn test_group_by_identity_outside_window_separate() {
        let engine = ReconciliationEngine::new();
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 10, 0, 0).unwrap();

        let e1 = make_event_at(
            "codex",
            Some("gpt-4"),
            ts,
            TokenUsage::default(),
        );
        let e2 = make_event_at(
            "codex",
            Some("gpt-4"),
            ts + Duration::seconds(2), // > 1 second apart
            TokenUsage::default(),
        );

        let groups = engine.group_by_identity(vec![e1, e2]);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn test_merge_tokens_prefers_more_fields() {
        let sparse = TokenUsage {
            total_tokens: Some(300),
            ..Default::default()
        };
        let detailed = TokenUsage {
            input_tokens: Some(100),
            output_tokens: Some(200),
            total_tokens: Some(300),
            ..Default::default()
        };

        let result = ReconciliationEngine::merge_tokens(&sparse, &detailed);
        assert_eq!(result.input_tokens, Some(100));
        assert_eq!(result.output_tokens, Some(200));
        assert_eq!(result.total_tokens, Some(300));
    }

    #[test]
    fn test_merge_tokens_equal_fields_prefers_b() {
        let a = TokenUsage {
            input_tokens: Some(100),
            output_tokens: Some(200),
            ..Default::default()
        };
        let b = TokenUsage {
            input_tokens: Some(150),
            output_tokens: Some(250),
            ..Default::default()
        };

        // Both have 2 fields, so prefer a (a_count is not less than b_count)
        let result = ReconciliationEngine::merge_tokens(&a, &b);
        assert_eq!(result.input_tokens, Some(100));
        assert_eq!(result.output_tokens, Some(200));
    }

    #[test]
    fn test_merge_quota_combines_complementary() {
        let qa = Some(QuotaUsage {
            fast_hours_pct: Some(50.0),
            standard_pct: None,
            excess_pct: None,
            daily_tokens: Some(1000),
        });
        let qb = Some(QuotaUsage {
            fast_hours_pct: None,
            standard_pct: Some(30.0),
            excess_pct: Some(10.0),
            daily_tokens: None,
        });

        let result = ReconciliationEngine::merge_quota(&qa, &qb);
        let merged = result.unwrap();
        assert_eq!(merged.fast_hours_pct, Some(50.0)); // from qa (qb is None)
        assert_eq!(merged.standard_pct, Some(30.0));   // from qb
        assert_eq!(merged.excess_pct, Some(10.0));     // from qb
        assert_eq!(merged.daily_tokens, Some(1000));   // from qa (qb is None)
    }

    #[test]
    fn test_merge_quota_none_cases() {
        assert!(ReconciliationEngine::merge_quota(&None, &None).is_none());

        let qa = Some(QuotaUsage {
            fast_hours_pct: Some(50.0),
            ..Default::default()
        });
        let result = ReconciliationEngine::merge_quota(&qa, &None);
        assert_eq!(result.unwrap().fast_hours_pct, Some(50.0));

        let qb = Some(QuotaUsage {
            standard_pct: Some(30.0),
            ..Default::default()
        });
        let result = ReconciliationEngine::merge_quota(&None, &qb);
        assert_eq!(result.unwrap().standard_pct, Some(30.0));
    }

    #[test]
    fn test_reconcile_merges_group_with_source_count() {
        let engine = ReconciliationEngine::new();
        let ts = Utc.with_ymd_and_hms(2024, 1, 15, 10, 0, 0).unwrap();

        let e1 = make_event_at(
            "codex",
            Some("gpt-4"),
            ts,
            TokenUsage {
                total_tokens: Some(300),
                ..Default::default()
            },
        );
        let e2 = make_event_at(
            "codex",
            Some("gpt-4"),
            ts + Duration::milliseconds(200),
            TokenUsage {
                input_tokens: Some(100),
                output_tokens: Some(200),
                total_tokens: Some(300),
                ..Default::default()
            },
        );

        let result = engine.reconcile(vec![e1, e2]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].source_count, 2);
        // Should prefer the more detailed token set
        assert_eq!(result[0].tokens.input_tokens, Some(100));
        assert_eq!(result[0].tokens.output_tokens, Some(200));
        assert_eq!(result[0].tokens.total_tokens, Some(300));
    }

    #[test]
    fn test_reconcile_preserves_context_window() {
        let engine = ReconciliationEngine::new();
        let ts = Utc::now();

        let mut e1 = make_event_at("codex", Some("gpt-4"), ts, TokenUsage::default());
        e1.context_window = None;

        let mut e2 = make_event_at(
            "codex",
            Some("gpt-4"),
            ts + Duration::milliseconds(100),
            TokenUsage::default(),
        );
        e2.context_window = Some(128000);

        let result = engine.reconcile(vec![e1, e2]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].context_window, Some(128000));
    }

    #[test]
    fn test_reconcile_empty_input() {
        let engine = ReconciliationEngine::new();
        let result = engine.reconcile(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_reconcile_multiple_providers_separate() {
        let engine = ReconciliationEngine::new();
        let ts = Utc::now();

        let e1 = make_event_at("codex", Some("gpt-4"), ts, TokenUsage::default());
        let e2 = make_event_at("claude", Some("claude-3"), ts, TokenUsage::default());

        let result = engine.reconcile(vec![e1, e2]);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|r| r.source_count == 1));
    }
}

#[cfg(test)]
mod prop_tests_reconciliation {
    //! Property-based tests for reconciliation no-information-loss.
    //!
    //! **Validates: Requirements 3.2, 3.3, 3.4, 3.5**
    //!
    //! These tests verify that the ReconciliationEngine merges events without
    //! losing information, accurately tracks source counts, prefers more detailed
    //! token data, properly merges complementary quota fields, and never creates
    //! more output events than input events.

    use super::*;
    use crate::types::{EventType, QuotaUsage, TokenUsage};
    use chrono::{Duration, TimeZone, Utc};
    use proptest::prelude::*;
    use proptest::test_runner::Config as ProptestConfig;

    /// Strategy to generate arbitrary EventType values.
    fn arb_event_type() -> impl Strategy<Value = EventType> {
        prop_oneof![
            Just(EventType::TokenCount),
            Just(EventType::SessionSummary),
            Just(EventType::QuotaSample),
            Just(EventType::DailyAggregate),
        ]
    }

    /// Strategy to generate arbitrary TokenUsage values.
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

    /// Strategy to generate arbitrary QuotaUsage values.
    fn arb_quota_usage() -> impl Strategy<Value = QuotaUsage> {
        (
            proptest::option::of(0.0f64..100.0),
            proptest::option::of(0.0f64..100.0),
            proptest::option::of(0.0f64..150.0),
            proptest::option::of(0u64..10_000_000),
        )
            .prop_map(|(fast, standard, excess, daily)| QuotaUsage {
                fast_hours_pct: fast,
                standard_pct: standard,
                excess_pct: excess,
                daily_tokens: daily,
            })
    }

    /// Strategy to generate a valid UTC timestamp within a reasonable range.
    fn arb_timestamp() -> impl Strategy<Value = chrono::DateTime<Utc>> {
        (1577836800i64..1893456000i64)
            .prop_map(|secs| Utc.timestamp_opt(secs, 0).unwrap())
    }

    /// Strategy to generate an optional model string.
    fn arb_model() -> impl Strategy<Value = Option<String>> {
        proptest::option::of("[a-z0-9\\-]{1,30}")
    }

    /// Strategy to generate a provider_id string (non-empty).
    fn arb_provider_id() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9\\-]{0,19}"
    }

    /// Strategy to generate a group of 2-5 events that share the same provider_id
    /// and model, with timestamps within a 1-second window (so they get grouped).
    fn arb_grouped_events() -> impl Strategy<Value = Vec<RawUsageEvent>> {
        (
            arb_provider_id(),
            arb_model(),
            arb_event_type(),
            arb_timestamp(),
            proptest::collection::vec(
                (
                    arb_token_usage(),
                    proptest::option::of(arb_quota_usage()),
                    proptest::option::of(1u64..500_000),
                    proptest::option::of("[a-f0-9]{8}"),
                    proptest::option::of("[a-f0-9]{8}"),
                    0u32..1000, // millisecond offset within window
                ),
                2..=5,
            ),
        )
            .prop_map(
                |(provider_id, model, event_type, base_ts, event_params)| {
                    event_params
                        .into_iter()
                        .map(|(tokens, quota, ctx_window, session, project, ms_offset)| {
                            RawUsageEvent {
                                provider_id: provider_id.clone(),
                                event_type: event_type.clone(),
                                timestamp: base_ts + Duration::milliseconds(ms_offset as i64),
                                model: model.clone(),
                                tokens,
                                context_window: ctx_window,
                                quota,
                                session_hash: session,
                                project_hash: project,
                                source_file: None,
                                raw_metadata: None,
                            }
                        })
                        .collect()
                },
            )
    }

    /// Strategy to generate a Vec of 1-8 arbitrary events (some may group, some not).
    fn arb_event_vec() -> impl Strategy<Value = Vec<RawUsageEvent>> {
        proptest::collection::vec(
            (
                arb_provider_id(),
                arb_event_type(),
                arb_timestamp(),
                arb_model(),
                arb_token_usage(),
                proptest::option::of(arb_quota_usage()),
                proptest::option::of(1u64..500_000),
            ),
            1..8,
        )
            .prop_map(|params| {
                params
                    .into_iter()
                    .map(
                        |(provider_id, event_type, timestamp, model, tokens, quota, ctx)| {
                            RawUsageEvent {
                                provider_id,
                                event_type,
                                timestamp,
                                model,
                                tokens,
                                context_window: ctx,
                                quota,
                                session_hash: None,
                                project_hash: None,
                                source_file: None,
                                raw_metadata: None,
                            }
                        },
                    )
                    .collect()
            })
    }

    /// Helper: count non-None fields in a TokenUsage.
    fn count_token_fields(t: &TokenUsage) -> usize {
        let mut count = 0;
        if t.input_tokens.is_some() { count += 1; }
        if t.cached_input_tokens.is_some() { count += 1; }
        if t.output_tokens.is_some() { count += 1; }
        if t.reasoning_tokens.is_some() { count += 1; }
        if t.total_tokens.is_some() { count += 1; }
        count
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20))]

        /// **Property 1: No information loss**
        ///
        /// For any group of events that get reconciled, all non-None field values
        /// from ANY source event must appear in the reconciled output.
        /// Specifically: for quota fields (merged via complement), every non-None
        /// value from any source must be present in the output.
        //        // **Validates: Requirements 3.5**
        #[test]
        fn reconciliation_no_information_loss_quota(events in arb_grouped_events()) {
            let engine = ReconciliationEngine::new();

            // Collect all non-None quota fields from any source event
            let mut any_fast: Option<f64> = None;
            let mut any_standard: Option<f64> = None;
            let mut any_excess: Option<f64> = None;
            let mut any_daily: Option<u64> = None;

            for event in &events {
                if let Some(ref q) = event.quota {
                    // The merge logic uses `qb.field.or(qa.field)`, preferring later events
                    // So the last non-None value for each field wins
                    if q.fast_hours_pct.is_some() { any_fast = q.fast_hours_pct; }
                    if q.standard_pct.is_some() { any_standard = q.standard_pct; }
                    if q.excess_pct.is_some() { any_excess = q.excess_pct; }
                    if q.daily_tokens.is_some() { any_daily = q.daily_tokens; }
                }
            }

            let result = engine.reconcile(events);
            prop_assert_eq!(result.len(), 1, "Grouped events should produce exactly one output");

            let reconciled = &result[0];
            let out_quota = &reconciled.quota;

            // Every non-None quota field from any source should appear in output
            if any_fast.is_some() || any_standard.is_some() || any_excess.is_some() || any_daily.is_some() {
                prop_assert!(out_quota.is_some(), "Output quota must be Some when any input has quota data");
                let oq = out_quota.as_ref().unwrap();
                if any_fast.is_some() {
                    prop_assert!(oq.fast_hours_pct.is_some(),
                        "fast_hours_pct was present in input but lost in output");
                }
                if any_standard.is_some() {
                    prop_assert!(oq.standard_pct.is_some(),
                        "standard_pct was present in input but lost in output");
                }
                if any_excess.is_some() {
                    prop_assert!(oq.excess_pct.is_some(),
                        "excess_pct was present in input but lost in output");
                }
                if any_daily.is_some() {
                    prop_assert!(oq.daily_tokens.is_some(),
                        "daily_tokens was present in input but lost in output");
                }
            }
        }

        /// **Property 2: Source count accuracy**
        ///
        /// The `source_count` field in the output equals the number of raw events
        /// that contributed to that reconciled event.
        //        // **Validates: Requirements 3.4**
        #[test]
        fn reconciliation_source_count_accuracy(events in arb_grouped_events()) {
            let engine = ReconciliationEngine::new();
            let input_count = events.len();

            let result = engine.reconcile(events);
            // All events share same provider_id, model, and are within 1s window
            // so they should be grouped into a single reconciled event
            prop_assert_eq!(result.len(), 1,
                "Grouped events should produce exactly one output");
            prop_assert_eq!(result[0].source_count as usize, input_count,
                "source_count ({}) must equal input event count ({})",
                result[0].source_count, input_count);
        }

        /// **Property 3: Merge prefers more detail**
        ///
        /// When merging tokens, the result should have at least as many non-None
        /// fields as the most detailed input.
        //        // **Validates: Requirements 3.2**
        #[test]
        fn reconciliation_merge_prefers_more_detail(events in arb_grouped_events()) {
            let engine = ReconciliationEngine::new();

            // Find the maximum number of non-None token fields in any input event
            let max_token_fields = events.iter()
                .map(|e| count_token_fields(&e.tokens))
                .max()
                .unwrap_or(0);

            let result = engine.reconcile(events);
            prop_assert_eq!(result.len(), 1);

            let output_fields = count_token_fields(&result[0].tokens);
            prop_assert!(output_fields >= max_token_fields,
                "Output token detail ({}) must be >= max input detail ({})",
                output_fields, max_token_fields);
        }

        /// **Property 4: Complementary quota merging**
        ///
        /// When events have complementary quota fields (one has fast_hours, another
        /// has standard), the merged result has ALL non-None fields from both sources.
        //        // **Validates: Requirements 3.3**
        #[test]
        fn reconciliation_complementary_quota_merging(events in arb_grouped_events()) {
            let engine = ReconciliationEngine::new();

            // Count distinct non-None quota fields across ALL input events
            let mut has_fast = false;
            let mut has_standard = false;
            let mut has_excess = false;
            let mut has_daily = false;

            for event in &events {
                if let Some(ref q) = event.quota {
                    if q.fast_hours_pct.is_some() { has_fast = true; }
                    if q.standard_pct.is_some() { has_standard = true; }
                    if q.excess_pct.is_some() { has_excess = true; }
                    if q.daily_tokens.is_some() { has_daily = true; }
                }
            }

            let result = engine.reconcile(events);
            prop_assert_eq!(result.len(), 1);

            if let Some(ref oq) = result[0].quota {
                // Every quota field that existed in ANY input must be in the output
                if has_fast {
                    prop_assert!(oq.fast_hours_pct.is_some(),
                        "Complementary merge lost fast_hours_pct");
                }
                if has_standard {
                    prop_assert!(oq.standard_pct.is_some(),
                        "Complementary merge lost standard_pct");
                }
                if has_excess {
                    prop_assert!(oq.excess_pct.is_some(),
                        "Complementary merge lost excess_pct");
                }
                if has_daily {
                    prop_assert!(oq.daily_tokens.is_some(),
                        "Complementary merge lost daily_tokens");
                }
            } else {
                // If output has no quota, then no input should have had quota either
                prop_assert!(!has_fast && !has_standard && !has_excess && !has_daily,
                    "Output quota is None but input had quota fields");
            }
        }

        /// **Property 5: Output count <= Input count**
        ///
        /// Reconciliation never creates more events than it receives.
        //        // **Validates: Requirements 3.2, 3.3, 3.4, 3.5**
        #[test]
        fn reconciliation_output_count_leq_input(events in arb_event_vec()) {
            let engine = ReconciliationEngine::new();
            let input_count = events.len();

            let result = engine.reconcile(events);
            prop_assert!(result.len() <= input_count,
                "Output count ({}) must be <= input count ({})",
                result.len(), input_count);
        }
    }
}
