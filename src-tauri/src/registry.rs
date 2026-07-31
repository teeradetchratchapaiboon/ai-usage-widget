use crate::error::CollectionError;
use crate::provider::{CollectionResult, ProviderAdapter, ProviderSummary};
use chrono::{DateTime, Utc};

/// Outcome of a single provider's collection attempt within collect_all.
#[derive(Debug)]
pub enum ProviderCollectionOutcome {
    /// Collection succeeded with results.
    Success {
        provider_id: String,
        result: CollectionResult,
    },
    /// Collection failed but was isolated; other providers are unaffected.
    Failed {
        provider_id: String,
        error: CollectionError,
    },
}

/// Registry managing all registered provider adapters.
///
/// The registry orchestrates collection across providers with error isolation:
/// a failure in one provider never prevents others from collecting.
pub struct ProviderRegistry {
    adapters: Vec<Box<dyn ProviderAdapter>>,
}

impl ProviderRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
        }
    }

    /// Registers a provider adapter with the registry.
    pub fn register(&mut self, adapter: Box<dyn ProviderAdapter>) {
        self.adapters.push(adapter);
    }

    /// Returns the number of registered adapters.
    pub fn adapter_count(&self) -> usize {
        self.adapters.len()
    }

    /// Collects data from all registered adapters, isolating failures.
    ///
    /// Each provider is collected independently. If one provider fails,
    /// the error is captured in the outcome but does not prevent other
    /// providers from being collected.
    pub fn collect_all(&self) -> Vec<ProviderCollectionOutcome> {
        self.adapters
            .iter()
            .map(|adapter| {
                let provider_id = adapter.provider_id().to_string();
                let since = adapter.last_checkpoint();

                match adapter.collect(since) {
                    Ok(result) => ProviderCollectionOutcome::Success {
                        provider_id,
                        result,
                    },
                    Err(error) => ProviderCollectionOutcome::Failed {
                        provider_id,
                        error,
                    },
                }
            })
            .collect()
    }

    /// Collects data from all registered adapters using a specified checkpoint.
    ///
    /// Same as `collect_all` but uses the provided `since` timestamp for all adapters.
    pub fn collect_all_since(&self, since: Option<DateTime<Utc>>) -> Vec<ProviderCollectionOutcome> {
        self.adapters
            .iter()
            .map(|adapter| {
                let provider_id = adapter.provider_id().to_string();

                match adapter.collect(since) {
                    Ok(result) => ProviderCollectionOutcome::Success {
                        provider_id,
                        result,
                    },
                    Err(error) => ProviderCollectionOutcome::Failed {
                        provider_id,
                        error,
                    },
                }
            })
            .collect()
    }

    /// Gets summaries from all registered providers.
    ///
    /// If a provider fails to produce a summary, a fallback summary
    /// is returned with is_available = false.
    pub fn get_all_summaries(&self) -> Vec<ProviderSummary> {
        self.adapters
            .iter()
            .map(|adapter| {
                adapter.get_current_summary().unwrap_or_else(|_| {
                    ProviderSummary {
                        provider_id: adapter.provider_id().to_string(),
                        display_name: adapter.display_name().to_string(),
                        is_available: false,
                        current_model: None,
                        tokens_today: None,
                        quota: None,
                        context_window: None,
                        last_activity: None,
                    }
                })
            })
            .collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod prop_tests_provider_isolation {
    use super::*;
    use crate::provider::{CollectionResult, ProviderAdapter, ProviderSummary, SourceMetadata};
    use crate::types::{EventType, RawUsageEvent, TokenUsage};
    use chrono::Utc;
    use proptest::prelude::*;
    use proptest::test_runner::Config;

    /// **Validates: Requirements 1.2, 16.1, 16.5**

    // --- Mock Providers ---

    /// A provider that always succeeds, returning configurable events.
    struct SuccessProvider {
        id: String,
        events: Vec<RawUsageEvent>,
    }

    impl SuccessProvider {
        fn new(id: String, event_count: usize) -> Self {
            let events = (0..event_count)
                .map(|i| RawUsageEvent {
                    provider_id: id.clone(),
                    event_type: EventType::TokenCount,
                    timestamp: Utc::now(),
                    model: Some(format!("model-{}", i)),
                    tokens: TokenUsage {
                        input_tokens: Some(100 * (i as u64 + 1)),
                        cached_input_tokens: None,
                        output_tokens: Some(50 * (i as u64 + 1)),
                        reasoning_tokens: None,
                        total_tokens: Some(150 * (i as u64 + 1)),
                    },
                    context_window: Some(128_000),
                    quota: None,
                    session_hash: None,
                    project_hash: None,
                    source_file: None,
                    raw_metadata: None,
                })
                .collect();
            Self { id, events }
        }
    }

    impl ProviderAdapter for SuccessProvider {
        fn provider_id(&self) -> &str {
            &self.id
        }
        fn display_name(&self) -> &str {
            &self.id
        }
        fn is_available(&self) -> bool {
            true
        }
        fn collect(&self, _since: Option<DateTime<Utc>>) -> Result<CollectionResult, CollectionError> {
            Ok(CollectionResult {
                events: self.events.clone(),
                checkpoint: Utc::now(),
                source_metadata: SourceMetadata {
                    files_read: 1,
                    bytes_processed: 1024,
                    errors: vec![],
                },
            })
        }
        fn get_current_summary(&self) -> Result<ProviderSummary, CollectionError> {
            Ok(ProviderSummary {
                provider_id: self.id.clone(),
                display_name: self.id.clone(),
                is_available: true,
                current_model: Some("test-model".to_string()),
                tokens_today: Some(1000),
                quota: None,
                context_window: Some(128_000),
                last_activity: Some(Utc::now()),
            })
        }
        fn last_checkpoint(&self) -> Option<DateTime<Utc>> {
            None
        }
    }

    /// A provider that always fails with a CollectionError.
    struct FailProvider {
        id: String,
        error_msg: String,
    }

    impl FailProvider {
        fn new(id: String, error_msg: String) -> Self {
            Self { id, error_msg }
        }
    }

    impl ProviderAdapter for FailProvider {
        fn provider_id(&self) -> &str {
            &self.id
        }
        fn display_name(&self) -> &str {
            &self.id
        }
        fn is_available(&self) -> bool {
            false
        }
        fn collect(&self, _since: Option<DateTime<Utc>>) -> Result<CollectionResult, CollectionError> {
            Err(CollectionError::DataSourceUnavailable(
                self.id.clone(),
                self.error_msg.clone(),
            ))
        }
        fn get_current_summary(&self) -> Result<ProviderSummary, CollectionError> {
            Err(CollectionError::DataSourceUnavailable(
                self.id.clone(),
                self.error_msg.clone(),
            ))
        }
        fn last_checkpoint(&self) -> Option<DateTime<Utc>> {
            None
        }
    }

    // --- Strategies ---

    /// Strategy to produce a random mix of success/fail providers.
    /// Each element is (is_success: bool, event_count: usize).
    fn provider_mix_strategy() -> impl Strategy<Value = Vec<(bool, usize)>> {
        prop::collection::vec((any::<bool>(), 0usize..5), 1..8)
    }

    // --- Property Tests ---

    proptest! {
        #![proptest_config(Config::with_cases(20))]

        /// Property 1: Error isolation - when one provider fails, other providers still
        /// succeed and produce their events. The registry never panics.
        #[test]
        fn error_isolation_other_providers_succeed(
            mix in provider_mix_strategy()
        ) {
            let mut registry = ProviderRegistry::new();

            for (i, (is_success, event_count)) in mix.iter().enumerate() {
                if *is_success {
                    registry.register(Box::new(SuccessProvider::new(
                        format!("success-{}", i),
                        *event_count,
                    )));
                } else {
                    registry.register(Box::new(FailProvider::new(
                        format!("fail-{}", i),
                        "test error".to_string(),
                    )));
                }
            }

            let outcomes = registry.collect_all();

            // Check each success provider returned its events correctly
            let mut outcome_idx = 0;
            for (i, (is_success, event_count)) in mix.iter().enumerate() {
                let outcome = &outcomes[outcome_idx];
                outcome_idx += 1;

                if *is_success {
                    match outcome {
                        ProviderCollectionOutcome::Success { provider_id, result } => {
                            prop_assert_eq!(provider_id, &format!("success-{}", i));
                            prop_assert_eq!(result.events.len(), *event_count);
                        }
                        ProviderCollectionOutcome::Failed { provider_id, .. } => {
                            return Err(proptest::test_runner::TestCaseError::Fail(
                                format!("Expected success for provider {} but got failure", provider_id).into()
                            ));
                        }
                    }
                } else {
                    match outcome {
                        ProviderCollectionOutcome::Failed { provider_id, .. } => {
                            prop_assert_eq!(provider_id, &format!("fail-{}", i));
                        }
                        ProviderCollectionOutcome::Success { provider_id, .. } => {
                            return Err(proptest::test_runner::TestCaseError::Fail(
                                format!("Expected failure for provider {} but got success", provider_id).into()
                            ));
                        }
                    }
                }
            }
        }

        /// Property 2: Never crashes - for any combination of N providers (some succeed,
        /// some fail), collect_all() always returns exactly N outcomes without panicking.
        #[test]
        fn collect_all_returns_exactly_n_outcomes(
            mix in provider_mix_strategy()
        ) {
            let mut registry = ProviderRegistry::new();
            let n = mix.len();

            for (i, (is_success, event_count)) in mix.iter().enumerate() {
                if *is_success {
                    registry.register(Box::new(SuccessProvider::new(
                        format!("provider-{}", i),
                        *event_count,
                    )));
                } else {
                    registry.register(Box::new(FailProvider::new(
                        format!("provider-{}", i),
                        "error".to_string(),
                    )));
                }
            }

            let outcomes = registry.collect_all();
            prop_assert_eq!(outcomes.len(), n);
        }

        /// Property 3: Success providers unaffected - the events from successful providers
        /// are exactly what those providers returned, regardless of how many others failed.
        #[test]
        fn success_events_unaffected_by_failures(
            num_success in 1usize..5,
            num_fail in 0usize..5,
            events_per_success in 1usize..4,
        ) {
            let mut registry = ProviderRegistry::new();

            // Register success providers first, then fail providers
            for i in 0..num_success {
                registry.register(Box::new(SuccessProvider::new(
                    format!("good-{}", i),
                    events_per_success,
                )));
            }
            for i in 0..num_fail {
                registry.register(Box::new(FailProvider::new(
                    format!("bad-{}", i),
                    format!("error-{}", i),
                )));
            }

            let outcomes = registry.collect_all();

            // Count successful outcomes and verify their event counts
            let success_outcomes: Vec<_> = outcomes.iter().filter_map(|o| match o {
                ProviderCollectionOutcome::Success { provider_id, result } => Some((provider_id, result)),
                _ => None,
            }).collect();

            prop_assert_eq!(success_outcomes.len(), num_success);

            for (_, result) in &success_outcomes {
                prop_assert_eq!(result.events.len(), events_per_success);
            }
        }

        /// Property 4: Failed outcomes captured - every failing provider appears in the
        /// output as a Failed variant with its provider_id and error preserved.
        #[test]
        fn failed_outcomes_captured_with_provider_id(
            num_success in 0usize..4,
            num_fail in 1usize..5,
        ) {
            let mut registry = ProviderRegistry::new();

            for i in 0..num_success {
                registry.register(Box::new(SuccessProvider::new(
                    format!("ok-{}", i),
                    1,
                )));
            }
            for i in 0..num_fail {
                registry.register(Box::new(FailProvider::new(
                    format!("err-{}", i),
                    format!("msg-{}", i),
                )));
            }

            let outcomes = registry.collect_all();

            let failed_outcomes: Vec<_> = outcomes.iter().filter_map(|o| match o {
                ProviderCollectionOutcome::Failed { provider_id, error } => {
                    Some((provider_id.clone(), format!("{}", error)))
                }
                _ => None,
            }).collect();

            prop_assert_eq!(failed_outcomes.len(), num_fail);

            for i in 0..num_fail {
                let expected_id = format!("err-{}", i);
                let expected_msg_fragment = format!("msg-{}", i);
                let found = failed_outcomes.iter().any(|(id, err_msg)| {
                    id == &expected_id && err_msg.contains(&expected_msg_fragment)
                });
                prop_assert!(found, "Expected failed outcome for {} not found", expected_id);
            }
        }

        /// Property 5: get_all_summaries fallback - when get_current_summary fails,
        /// the registry returns a fallback summary with is_available=false.
        #[test]
        fn get_all_summaries_fallback_on_failure(
            num_success in 0usize..4,
            num_fail in 1usize..5,
        ) {
            let mut registry = ProviderRegistry::new();

            for i in 0..num_success {
                registry.register(Box::new(SuccessProvider::new(
                    format!("ok-{}", i),
                    1,
                )));
            }
            for i in 0..num_fail {
                registry.register(Box::new(FailProvider::new(
                    format!("err-{}", i),
                    format!("summary-fail-{}", i),
                )));
            }

            let summaries = registry.get_all_summaries();

            // Always returns one summary per provider
            prop_assert_eq!(summaries.len(), num_success + num_fail);

            // Success providers have is_available = true
            let available: Vec<_> = summaries.iter().filter(|s| s.is_available).collect();
            prop_assert_eq!(available.len(), num_success);

            // Failed providers have fallback with is_available = false
            let unavailable: Vec<_> = summaries.iter().filter(|s| !s.is_available).collect();
            prop_assert_eq!(unavailable.len(), num_fail);

            // Verify fallback summaries have correct provider_id and no data
            for summary in &unavailable {
                prop_assert!(summary.provider_id.starts_with("err-"));
                prop_assert!(summary.current_model.is_none());
                prop_assert!(summary.tokens_today.is_none());
                prop_assert!(summary.quota.is_none());
                prop_assert!(summary.context_window.is_none());
                prop_assert!(summary.last_activity.is_none());
            }
        }
    }
}
