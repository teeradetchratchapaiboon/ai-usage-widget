use std::sync::Arc;

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::error::StorageError;
use crate::types::{EventFingerprint, EventType, RawUsageEvent};

impl EventType {
    /// Return a stable string representation for fingerprint hashing.
    pub fn as_str(&self) -> &str {
        match self {
            EventType::TokenCount => "token_count",
            EventType::SessionSummary => "session_summary",
            EventType::QuotaSample => "quota_sample",
            EventType::DailyAggregate => "daily_aggregate",
        }
    }
}

/// Compute a deterministic SHA-256 fingerprint for a RawUsageEvent.
///
/// The fingerprint is based on: provider_id, timestamp, event_type, model, and token values.
/// Same inputs will always produce the same fingerprint regardless of when computed.
pub fn compute_fingerprint(event: &RawUsageEvent) -> EventFingerprint {
    let mut hasher = Sha256::new();
    hasher.update(event.provider_id.as_bytes());
    hasher.update(event.timestamp.to_rfc3339().as_bytes());
    hasher.update(event.event_type.as_str().as_bytes());

    if let Some(ref model) = event.model {
        hasher.update(model.as_bytes());
    }

    if let Some(total) = event.tokens.total_tokens {
        hasher.update(&total.to_le_bytes());
    }
    if let Some(input) = event.tokens.input_tokens {
        hasher.update(&input.to_le_bytes());
    }
    if let Some(output) = event.tokens.output_tokens {
        hasher.update(&output.to_le_bytes());
    }

    let result: [u8; 32] = hasher.finalize().into();
    EventFingerprint(result)
}

/// Two-phase deduplication engine using a Bloom filter for fast rejection
/// and SQLite for definitive verification.
pub struct DeduplicationEngine {
    seen_fingerprints: bloomfilter::Bloom<[u8; 32]>,
    db: Arc<SqlitePool>,
}

impl DeduplicationEngine {
    /// Create a new DeduplicationEngine, preloading the Bloom filter from existing
    /// fingerprints in the database.
    ///
    /// Bloom filter is configured for ~1M items with ~1% false positive rate.
    pub async fn new(db: Arc<SqlitePool>) -> Result<Self, StorageError> {
        // Bloom filter parameters:
        // - Expected items: 1,000,000
        // - False positive rate: ~1% (0.01)
        // bloomfilter::Bloom::new_for_fp_rate(expected_items, fp_rate)
        let mut bloom = bloomfilter::Bloom::new_for_fp_rate(1_000_000, 0.01);

        // Preload existing fingerprints from database
        let rows: Vec<(Vec<u8>,)> =
            sqlx::query_as("SELECT fingerprint FROM seen_fingerprints")
                .fetch_all(db.as_ref())
                .await
                .map_err(|e| {
                    StorageError::QueryFailed(format!(
                        "failed to preload fingerprints: {}",
                        e
                    ))
                })?;

        for (fp_bytes,) in &rows {
            if fp_bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(fp_bytes);
                bloom.set(&arr);
            }
        }

        Ok(Self {
            seen_fingerprints: bloom,
            db,
        })
    }

    /// Deduplicate a list of raw events using two-phase checking:
    ///
    /// Phase 1: Bloom filter check (fast, no false negatives, may have false positives)
    /// Phase 2: SQLite verification for Bloom positives (definitive)
    ///
    /// Returns only events that are NOT already seen (unique events).
    pub async fn deduplicate(
        &self,
        events: Vec<RawUsageEvent>,
    ) -> Result<Vec<(RawUsageEvent, EventFingerprint)>, StorageError> {
        let mut unique_events = Vec::new();

        for event in events {
            let fingerprint = compute_fingerprint(&event);

            // Phase 1: Bloom filter check
            if self.seen_fingerprints.check(&fingerprint.0) {
                // Bloom says "maybe seen" — verify with SQLite (Phase 2)
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM seen_fingerprints WHERE fingerprint = ?)",
                )
                .bind(fingerprint.0.as_slice())
                .fetch_one(self.db.as_ref())
                .await
                .map_err(|e| {
                    StorageError::QueryFailed(format!(
                        "failed to verify fingerprint: {}",
                        e
                    ))
                })?;

                if !exists {
                    // False positive from Bloom filter — event is unique
                    unique_events.push((event, fingerprint));
                }
                // If exists == true, event is a duplicate — skip it
            } else {
                // Bloom says "definitely not seen" — event is unique
                unique_events.push((event, fingerprint));
            }
        }

        Ok(unique_events)
    }

    /// Register fingerprints as seen after events have been stored.
    ///
    /// Inserts into both the Bloom filter (for fast future checks) and SQLite
    /// (for definitive verification).
    pub async fn mark_seen(
        &mut self,
        fingerprints: &[EventFingerprint],
    ) -> Result<(), StorageError> {
        for fp in fingerprints {
            // Insert into SQLite
            sqlx::query(
                "INSERT OR IGNORE INTO seen_fingerprints (fingerprint) VALUES (?)",
            )
            .bind(fp.0.as_slice())
            .execute(self.db.as_ref())
            .await
            .map_err(|e| {
                StorageError::QueryFailed(format!(
                    "failed to insert fingerprint: {}",
                    e
                ))
            })?;

            // Update Bloom filter
            self.seen_fingerprints.set(&fp.0);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use crate::types::TokenUsage;

    fn make_event(provider: &str, model: Option<&str>, total_tokens: Option<u64>) -> RawUsageEvent {
        RawUsageEvent {
            provider_id: provider.to_string(),
            event_type: EventType::TokenCount,
            timestamp: Utc::now(),
            model: model.map(|s| s.to_string()),
            tokens: TokenUsage {
                input_tokens: Some(100),
                cached_input_tokens: None,
                output_tokens: Some(200),
                reasoning_tokens: None,
                total_tokens,
            },
            context_window: None,
            quota: None,
            session_hash: None,
            project_hash: None,
            source_file: None,
            raw_metadata: None,
        }
    }

    #[test]
    fn test_compute_fingerprint_deterministic() {
        let event = make_event("codex", Some("gpt-4"), Some(300));
        let fp1 = compute_fingerprint(&event);
        let fp2 = compute_fingerprint(&event);
        assert_eq!(fp1, fp2, "Same event must produce identical fingerprints");
    }

    #[test]
    fn test_compute_fingerprint_different_events() {
        let event1 = make_event("codex", Some("gpt-4"), Some(300));
        let event2 = make_event("claude", Some("claude-3"), Some(500));
        let fp1 = compute_fingerprint(&event1);
        let fp2 = compute_fingerprint(&event2);
        assert_ne!(fp1, fp2, "Different events must produce different fingerprints");
    }

    #[test]
    fn test_compute_fingerprint_none_model() {
        let event1 = make_event("codex", None, Some(300));
        let event2 = make_event("codex", Some("gpt-4"), Some(300));
        let fp1 = compute_fingerprint(&event1);
        let fp2 = compute_fingerprint(&event2);
        assert_ne!(fp1, fp2, "None model vs Some model must differ");
    }

    #[test]
    fn test_event_type_as_str() {
        assert_eq!(EventType::TokenCount.as_str(), "token_count");
        assert_eq!(EventType::SessionSummary.as_str(), "session_summary");
        assert_eq!(EventType::QuotaSample.as_str(), "quota_sample");
        assert_eq!(EventType::DailyAggregate.as_str(), "daily_aggregate");
    }
}

#[cfg(test)]
mod prop_tests_dedup_completeness {
    //! Property-based tests for deduplication completeness.
    //!
    //! **Validates: Requirements 2.2, 2.3**
    //!
    //! These tests verify that the deduplication engine correctly rejects events whose
    //! fingerprints already exist in the database, passes through unique events,
    //! is idempotent, produces a subset of the input, and preserves ordering.

    use super::*;
    use crate::types::TokenUsage;
    use chrono::{TimeZone, Utc};
    use proptest::prelude::*;
    use proptest::test_runner::Config as ProptestConfig;
    use std::collections::HashSet;
    use std::sync::Arc;

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

    /// Strategy to generate a full arbitrary RawUsageEvent.
    fn arb_raw_usage_event() -> impl Strategy<Value = RawUsageEvent> {
        (
            arb_provider_id(),
            arb_event_type(),
            arb_timestamp(),
            arb_model(),
            arb_token_usage(),
        )
            .prop_map(|(provider_id, event_type, timestamp, model, tokens)| {
                RawUsageEvent {
                    provider_id,
                    event_type,
                    timestamp,
                    model,
                    tokens,
                    context_window: None,
                    quota: None,
                    session_hash: None,
                    project_hash: None,
                    source_file: None,
                    raw_metadata: None,
                }
            })
    }

    /// Strategy to generate a Vec of 1-10 arbitrary events.
    fn arb_event_vec() -> impl Strategy<Value = Vec<RawUsageEvent>> {
        proptest::collection::vec(arb_raw_usage_event(), 1..10)
    }

    /// Helper: create an in-memory SQLite pool with the seen_fingerprints table.
    async fn setup_test_db() -> Arc<SqlitePool> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("failed to create in-memory SQLite pool");

        sqlx::query(
            "CREATE TABLE seen_fingerprints (fingerprint BLOB PRIMARY KEY, first_seen_at TEXT NOT NULL DEFAULT (datetime('now')))"
        )
        .execute(&pool)
        .await
        .expect("failed to create seen_fingerprints table");

        Arc::new(pool)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20))]

        /// **Property: Completeness — already-stored events are always rejected.**
        ///
        /// After marking a set of events as seen, deduplicating the same events
        /// must return an empty result (all are rejected).
        ///
        /// **Validates: Requirements 2.2, 2.3**
        #[test]
        fn dedup_completeness_rejects_stored_events(events in arb_event_vec()) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let result = rt.block_on(async {
                let db = setup_test_db().await;
                let mut engine = DeduplicationEngine::new(db.clone()).await.unwrap();

                // Mark all events as seen
                let fingerprints: Vec<EventFingerprint> = events.iter()
                    .map(|e| compute_fingerprint(e))
                    .collect();
                engine.mark_seen(&fingerprints).await.unwrap();

                // Deduplicate the same events — all should be rejected
                engine.deduplicate(events).await.unwrap()
            });
            prop_assert!(result.is_empty(),
                "All previously stored events must be rejected, but got {} through", result.len());
        }

        /// **Property: No false rejection — unique events are never lost.**
        ///
        /// If no events have been marked as seen, then deduplication must pass
        /// through all input events (none are falsely rejected).
        ///
        /// **Validates: Requirements 2.2, 2.3**
        #[test]
        fn dedup_no_false_rejection(events in arb_event_vec()) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let (output_fps, expected_unique_fps) = rt.block_on(async {
                let db = setup_test_db().await;
                let engine = DeduplicationEngine::new(db.clone()).await.unwrap();

                // No events have been marked as seen — all should pass through
                let result = engine.deduplicate(events.clone()).await.unwrap();

                // Collect fingerprints of input and output for comparison
                let input_fps: Vec<EventFingerprint> = events.iter()
                    .map(|e| compute_fingerprint(e))
                    .collect();
                let output_fps: Vec<EventFingerprint> = result.iter()
                    .map(|(_, fp)| fp.clone())
                    .collect();

                // Every input fingerprint that is unique should appear in output
                let mut seen_fps = HashSet::new();
                let mut expected_unique_fps = Vec::new();
                for fp in &input_fps {
                    if seen_fps.insert(fp.clone()) {
                        expected_unique_fps.push(fp.clone());
                    }
                }

                (output_fps, expected_unique_fps)
            });
            prop_assert_eq!(output_fps.len(), expected_unique_fps.len(),
                "All unique events must pass through when none are stored. Expected {}, got {}",
                expected_unique_fps.len(), output_fps.len());
        }

        /// **Property: Idempotency — deduplicating twice after mark_seen yields empty.**
        ///
        /// Running deduplication on a batch, marking them as seen, then deduplicating
        /// the same batch again must produce an empty result.
        ///
        /// **Validates: Requirements 2.2, 2.3**
        #[test]
        fn dedup_idempotency(events in arb_event_vec()) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let second_result = rt.block_on(async {
                let db = setup_test_db().await;
                let mut engine = DeduplicationEngine::new(db.clone()).await.unwrap();

                // First pass: deduplicate and mark seen
                let first_result = engine.deduplicate(events.clone()).await.unwrap();
                let first_fps: Vec<EventFingerprint> = first_result.iter()
                    .map(|(_, fp)| fp.clone())
                    .collect();
                engine.mark_seen(&first_fps).await.unwrap();

                // Second pass: deduplicate same events again — should be empty
                engine.deduplicate(events).await.unwrap()
            });
            prop_assert!(second_result.is_empty(),
                "Second deduplication after mark_seen must return empty, but got {} events",
                second_result.len());
        }

        /// **Property: Subset — the output is always a subset of the input.**
        ///
        /// No events are created out of thin air; every output event's fingerprint
        /// must match one of the input events' fingerprints.
        ///
        /// **Validates: Requirements 2.2, 2.3**
        #[test]
        fn dedup_output_is_subset_of_input(events in arb_event_vec()) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let (result, input_fps) = rt.block_on(async {
                let db = setup_test_db().await;
                let engine = DeduplicationEngine::new(db.clone()).await.unwrap();

                let input_fps: HashSet<EventFingerprint> = events.iter()
                    .map(|e| compute_fingerprint(e))
                    .collect();

                let result = engine.deduplicate(events).await.unwrap();
                (result, input_fps)
            });

            for (_, fp) in &result {
                prop_assert!(input_fps.contains(fp),
                    "Output event fingerprint {:?} not found in input — events were created from nothing",
                    fp.to_hex());
            }

            // Also verify output length is <= input length
            prop_assert!(result.len() <= input_fps.len(),
                "Output length {} exceeds number of unique input fingerprints {}",
                result.len(), input_fps.len());
        }

        /// **Property: Ordering preservation — output preserves input order of non-duplicate events.**
        ///
        /// The relative order of events in the output must match their order in the input.
        ///
        /// **Validates: Requirements 2.2, 2.3**
        #[test]
        fn dedup_preserves_ordering(events in arb_event_vec()) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let output_original_indices = rt.block_on(async {
                let db = setup_test_db().await;
                let engine = DeduplicationEngine::new(db.clone()).await.unwrap();

                // Compute fingerprints with their original indices
                let input_fps_with_idx: Vec<(usize, EventFingerprint)> = events.iter()
                    .enumerate()
                    .map(|(i, e)| (i, compute_fingerprint(e)))
                    .collect();

                let result = engine.deduplicate(events).await.unwrap();

                // Map output fingerprints to their original input indices
                let output_original_indices: Vec<usize> = result.iter()
                    .map(|(_, fp)| {
                        input_fps_with_idx.iter()
                            .find(|(_, ifp)| ifp == fp)
                            .map(|(idx, _)| *idx)
                            .expect("output fp must exist in input")
                    })
                    .collect();

                output_original_indices
            });

            // Verify the indices are in strictly increasing order
            for window in output_original_indices.windows(2) {
                prop_assert!(window[0] < window[1],
                    "Output ordering violated: index {} came before {} but should be after",
                    window[0], window[1]);
            }
        }
    }
}

#[cfg(test)]
mod prop_tests {
    //! Property-based tests for fingerprint determinism.
    //!
    //! **Validates: Requirements 2.1, 2.5**
    //!
    //! These tests verify that the SHA-256 fingerprint computation is deterministic,
    //! stable across time, and sensitive to all identity fields.

    use super::*;
    use crate::types::TokenUsage;
    use chrono::{TimeZone, Utc};
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

    /// Strategy to generate a valid UTC timestamp within a reasonable range.
    fn arb_timestamp() -> impl Strategy<Value = chrono::DateTime<Utc>> {
        // Range: 2020-01-01 to 2030-01-01 in seconds
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

    /// Strategy to generate a full arbitrary RawUsageEvent.
    fn arb_raw_usage_event() -> impl Strategy<Value = RawUsageEvent> {
        (
            arb_provider_id(),
            arb_event_type(),
            arb_timestamp(),
            arb_model(),
            arb_token_usage(),
        )
            .prop_map(|(provider_id, event_type, timestamp, model, tokens)| {
                RawUsageEvent {
                    provider_id,
                    event_type,
                    timestamp,
                    model,
                    tokens,
                    context_window: None,
                    quota: None,
                    session_hash: None,
                    project_hash: None,
                    source_file: None,
                    raw_metadata: None,
                }
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20))]

        /// **Property: Determinism**
        /// For any arbitrary RawUsageEvent, computing the fingerprint twice always
        /// produces the same result.
        ///
        /// **Validates: Requirements 2.1, 2.5**
        #[test]
        fn fingerprint_is_deterministic(event in arb_raw_usage_event()) {
            let fp1 = compute_fingerprint(&event);
            let fp2 = compute_fingerprint(&event);
            prop_assert_eq!(fp1, fp2, "Same event must always produce the same fingerprint");
        }

        /// **Property: Stability across time**
        /// The same event computed at different logical "times" (i.e., calling
        /// compute_fingerprint at different points) always produces the same fingerprint.
        /// This verifies the fingerprint does not incorporate the current wall clock.
        ///
        /// **Validates: Requirements 2.1, 2.5**
        #[test]
        fn fingerprint_stable_across_invocations(event in arb_raw_usage_event()) {
            let fp1 = compute_fingerprint(&event);
            // Clone the event to simulate receiving the same data at a different time
            let cloned_event = event.clone();
            let fp2 = compute_fingerprint(&cloned_event);
            prop_assert_eq!(fp1, fp2, "Fingerprint must be stable across separate invocations");
        }

        /// **Property: Input sensitivity — different provider_id**
        /// Two events that differ only in provider_id produce different fingerprints.
        ///
        /// **Validates: Requirements 2.1, 2.5**
        #[test]
        fn fingerprint_sensitive_to_provider_id(
            event in arb_raw_usage_event(),
            other_provider in "[a-z][a-z0-9\\-]{0,19}"
        ) {
            prop_assume!(event.provider_id != other_provider);
            let mut modified = event.clone();
            modified.provider_id = other_provider;

            let fp_original = compute_fingerprint(&event);
            let fp_modified = compute_fingerprint(&modified);
            prop_assert_ne!(fp_original, fp_modified,
                "Different provider_id must produce different fingerprints");
        }

        /// **Property: Input sensitivity — different timestamp**
        /// Two events that differ only in timestamp produce different fingerprints.
        ///
        /// **Validates: Requirements 2.1, 2.5**
        #[test]
        fn fingerprint_sensitive_to_timestamp(
            event in arb_raw_usage_event(),
            other_ts in arb_timestamp()
        ) {
            prop_assume!(event.timestamp != other_ts);
            let mut modified = event.clone();
            modified.timestamp = other_ts;

            let fp_original = compute_fingerprint(&event);
            let fp_modified = compute_fingerprint(&modified);
            prop_assert_ne!(fp_original, fp_modified,
                "Different timestamp must produce different fingerprints");
        }

        /// **Property: Input sensitivity — different event_type**
        /// Two events that differ only in event_type produce different fingerprints.
        ///
        /// **Validates: Requirements 2.1, 2.5**
        #[test]
        fn fingerprint_sensitive_to_event_type(
            event in arb_raw_usage_event(),
            other_type in arb_event_type()
        ) {
            prop_assume!(event.event_type != other_type);
            let mut modified = event.clone();
            modified.event_type = other_type;

            let fp_original = compute_fingerprint(&event);
            let fp_modified = compute_fingerprint(&modified);
            prop_assert_ne!(fp_original, fp_modified,
                "Different event_type must produce different fingerprints");
        }

        /// **Property: Input sensitivity — different model**
        /// Two events that differ in model (Some vs None, or different model strings)
        /// produce different fingerprints.
        ///
        /// **Validates: Requirements 2.1, 2.5**
        #[test]
        fn fingerprint_sensitive_to_model(
            event in arb_raw_usage_event(),
            other_model in arb_model()
        ) {
            prop_assume!(event.model != other_model);
            let mut modified = event.clone();
            modified.model = other_model;

            let fp_original = compute_fingerprint(&event);
            let fp_modified = compute_fingerprint(&modified);
            prop_assert_ne!(fp_original, fp_modified,
                "Different model must produce different fingerprints");
        }

        /// **Property: Input sensitivity — different token values**
        /// Two events that differ in total_tokens, input_tokens, or output_tokens
        /// produce different fingerprints.
        ///
        /// **Validates: Requirements 2.1, 2.5**
        #[test]
        fn fingerprint_sensitive_to_token_values(
            event in arb_raw_usage_event(),
            other_tokens in arb_token_usage()
        ) {
            // Only compare the fields that are used in fingerprinting:
            // total_tokens, input_tokens, output_tokens
            prop_assume!(
                event.tokens.total_tokens != other_tokens.total_tokens
                || event.tokens.input_tokens != other_tokens.input_tokens
                || event.tokens.output_tokens != other_tokens.output_tokens
            );
            let mut modified = event.clone();
            modified.tokens = other_tokens;

            let fp_original = compute_fingerprint(&event);
            let fp_modified = compute_fingerprint(&modified);
            prop_assert_ne!(fp_original, fp_modified,
                "Different token values must produce different fingerprints");
        }
    }
}
