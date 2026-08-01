use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;
use log::{error, info, warn};
use tokio::sync::Mutex;
use tokio::time::sleep;

use crate::dedup::DeduplicationEngine;
use crate::notify::{NotificationEngine, NotificationEntry, QuotaReading, QuotaWindow};
use crate::reconcile::ReconciliationEngine;
use crate::registry::{ProviderCollectionOutcome, ProviderRegistry};
use crate::storage::StorageLayer;

/// Adaptive threshold: if more than this many new events are collected,
/// temporarily reduce the interval.
const ADAPTIVE_THRESHOLD: usize = 50;

/// Temporary reduced interval (seconds) when high event volume is detected.
const ADAPTIVE_INTERVAL: u32 = 15;

/// Maximum backoff interval in seconds.
const MAX_BACKOFF: u32 = 300;

/// Scheduler that drives periodic data collection from all registered providers.
///
/// Features:
/// - Configurable default interval (default: 30 seconds)
/// - Exponential backoff on consecutive errors: min(30 * 2^N, 300) seconds
/// - Resets to default interval on first success after errors
/// - Adaptive interval: reduces to 15s temporarily if >50 new events collected
pub struct CollectionScheduler {
    /// Current interval in seconds.
    interval_secs: u32,
    /// Default interval (restored after errors clear).
    default_interval: u32,
    /// Number of consecutive collection cycles where ALL providers failed.
    consecutive_errors: u32,
    /// Shared stop signal for graceful shutdown.
    is_running: Arc<AtomicBool>,
    /// Quota threshold tracking (75% / 90% with per-provider cooldowns).
    notifications: NotificationEngine,
}

/// Sink invoked when a quota threshold is crossed.
pub type Notifier = Arc<dyn Fn(&NotificationEntry) + Send + Sync>;

impl CollectionScheduler {
    /// Create a new scheduler with the given default interval (in seconds).
    pub fn new(default_interval: u32) -> Self {
        Self {
            interval_secs: default_interval,
            default_interval,
            consecutive_errors: 0,
            is_running: Arc::new(AtomicBool::new(false)),
            notifications: NotificationEngine::new(),
        }
    }

    /// Start the collection loop. Call this inside a `tokio::spawn()`.
    ///
    /// The loop:
    /// 1. Sleeps for the current interval
    /// 2. Calls `registry.collect_all()`
    /// 3. Deduplicates, reconciles, and stores successful events
    /// 4. Adjusts interval based on results (adaptive or backoff)
    pub async fn run(
        &mut self,
        registry: Arc<ProviderRegistry>,
        dedup: Arc<Mutex<DeduplicationEngine>>,
        reconciliation: Arc<ReconciliationEngine>,
        storage: Arc<StorageLayer>,
    ) {
        self.run_with_notifier(registry, dedup, reconciliation, storage, None)
            .await
    }

    /// Same as [`run`](Self::run), plus quota-threshold notifications.
    ///
    /// `notifier` is called for every threshold crossing that is not in
    /// cooldown; the caller decides how to surface it (Windows toast in the
    /// app, a recording sink in tests).
    pub async fn run_with_notifier(
        &mut self,
        registry: Arc<ProviderRegistry>,
        dedup: Arc<Mutex<DeduplicationEngine>>,
        reconciliation: Arc<ReconciliationEngine>,
        storage: Arc<StorageLayer>,
        notifier: Option<Notifier>,
    ) {
        self.is_running.store(true, Ordering::SeqCst);

        info!(
            "Collection scheduler started with {}s default interval",
            self.default_interval
        );

        while self.is_running.load(Ordering::SeqCst) {
            // 1. Sleep for the current interval
            let interval = std::time::Duration::from_secs(self.interval_secs as u64);
            sleep(interval).await;

            // Check stop signal after waking
            if !self.is_running.load(Ordering::SeqCst) {
                break;
            }

            // 2. Collect from all providers (with error isolation per provider)
            let outcomes = registry.collect_all();

            // 3. Separate successes from failures
            let mut all_events = Vec::new();
            let mut any_success = false;
            let mut all_failed = true;

            for outcome in outcomes {
                match outcome {
                    ProviderCollectionOutcome::Success { provider_id, result } => {
                        info!(
                            "Provider '{}' collected {} events",
                            provider_id,
                            result.events.len()
                        );
                        all_events.extend(result.events);
                        any_success = true;
                        all_failed = false;
                    }
                    ProviderCollectionOutcome::Failed { provider_id, error } => {
                        warn!(
                            "Provider '{}' collection failed: {}",
                            provider_id, error
                        );
                    }
                }
            }

            // 4. Process events if any were collected
            let new_event_count = if !all_events.is_empty() {
                match self.process_events(all_events, &dedup, &reconciliation, &storage).await {
                    Ok(count) => count,
                    Err(e) => {
                        error!("Failed to process collected events: {}", e);
                        0
                    }
                }
            } else {
                0
            };

            // 4b. Persist incremental-collection state so a restart resumes
            // where this cycle stopped instead of re-reading every file.
            for (provider_id, state) in registry.export_states() {
                if state.file_positions.is_empty() && state.checkpoint.is_none() {
                    continue;
                }
                if let Err(e) = storage.save_provider_state(&provider_id, &state).await {
                    warn!("Failed to persist state for provider '{}': {}", provider_id, e);
                }
            }

            // 4c. Quota threshold notifications (75% / 90%, 1h cooldown each)
            if let Some(ref notify) = notifier {
                for summary in registry.get_all_summaries() {
                    let Some(quota) = summary.quota.as_ref() else {
                        continue;
                    };

                    // Each window is checked on its own rather than collapsed
                    // into their maximum. The maximum names no window, so the
                    // toast could not say whether the wait was hours or days,
                    // and a spent five-hour window masked the weekly one
                    // behind it entirely.
                    // One clock for the whole provider, so the two windows are
                    // judged against the same instant.
                    let now = Utc::now();
                    let (fast_timing, weekly_timing) = crate::commands::quota_timings(&summary);

                    let windows = [
                        (QuotaWindow::FastHours, quota.fast_hours_pct, fast_timing),
                        (QuotaWindow::Weekly, quota.standard_pct, weekly_timing),
                        // Excess carries no window of its own; it rides on the
                        // weekly reading that reported it.
                        (QuotaWindow::Excess, quota.excess_pct, weekly_timing),
                    ];

                    for (window, used_pct, timing) in windows {
                        let Some(used_pct) = used_pct else {
                            continue;
                        };

                        let entries = self.notifications.check_threshold(
                            &summary.provider_id,
                            &summary.display_name,
                            QuotaReading {
                                window,
                                used_pct,
                                resets_at: timing.resets_at,
                                estimated: summary.quota_resets.estimated,
                                freshness: timing.freshness(now),
                                age_secs: timing.age_seconds(now),
                            },
                        );
                        for entry in entries {
                            info!("Quota notification: {}", entry.message);
                            notify(&entry);
                        }
                    }
                }
            }

            // 5. Adjust interval based on outcome
            if all_failed && !any_success {
                // All providers failed — apply exponential backoff
                self.consecutive_errors += 1;
                self.interval_secs = self.backoff_interval();
                warn!(
                    "All providers failed (consecutive: {}). Next interval: {}s",
                    self.consecutive_errors, self.interval_secs
                );
            } else if any_success {
                // At least one provider succeeded — reset error counter
                if self.consecutive_errors > 0 {
                    info!(
                        "Collection recovered after {} consecutive errors",
                        self.consecutive_errors
                    );
                }
                self.consecutive_errors = 0;

                // Adaptive interval: if many events, collect sooner
                if new_event_count > ADAPTIVE_THRESHOLD {
                    self.interval_secs = ADAPTIVE_INTERVAL;
                    info!(
                        "High event volume ({} events). Reducing interval to {}s",
                        new_event_count, ADAPTIVE_INTERVAL
                    );
                } else {
                    self.interval_secs = self.default_interval;
                }
            }
        }

        info!("Collection scheduler stopped");
    }

    /// Replace the quota notification thresholds (percent).
    ///
    /// Existing cooldowns are kept, so raising a threshold does not immediately
    /// re-notify for a provider that already warned.
    pub fn set_notification_thresholds(&mut self, warning: f64, critical: f64) {
        self.notifications.set_thresholds(warning, critical);
    }

    /// Stop the collection loop gracefully.
    ///
    /// The loop will finish its current sleep cycle and then exit.
    pub fn stop(&self) {
        self.is_running.store(false, Ordering::SeqCst);
    }

    /// Calculate the next interval based on current state.
    ///
    /// Returns the adaptive interval (15s) if high volume was detected,
    /// the backoff interval if there are consecutive errors,
    /// or the default interval otherwise.
    pub fn calculate_interval(&self) -> u32 {
        if self.consecutive_errors > 0 {
            self.backoff_interval()
        } else {
            self.interval_secs
        }
    }

    /// Compute exponential backoff interval: min(default * 2^N, MAX_BACKOFF)
    /// where N = consecutive_errors.
    pub fn backoff_interval(&self) -> u32 {
        let backoff = self
            .default_interval
            .saturating_mul(2u32.saturating_pow(self.consecutive_errors));
        backoff.min(MAX_BACKOFF)
    }

    /// Process collected events through the dedup → reconcile → store pipeline.
    ///
    /// Returns the number of new events stored (after deduplication).
    async fn process_events(
        &self,
        events: Vec<crate::types::RawUsageEvent>,
        dedup: &Arc<Mutex<DeduplicationEngine>>,
        reconciliation: &Arc<ReconciliationEngine>,
        storage: &Arc<StorageLayer>,
    ) -> Result<usize, String> {
        process_events(events, dedup, reconciliation, storage).await
    }
}

/// Run collected events through dedup -> reconcile -> store.
///
/// Shared by the scheduler loop and the manual "Collect Now" command so both
/// paths persist what they collect.
pub async fn process_events(
    events: Vec<crate::types::RawUsageEvent>,
    dedup: &Arc<Mutex<DeduplicationEngine>>,
    reconciliation: &Arc<ReconciliationEngine>,
    storage: &Arc<StorageLayer>,
) -> Result<usize, String> {
        // Phase 1: Deduplicate
        let unique_events = {
            let dedup_guard = dedup.lock().await;
            dedup_guard
                .deduplicate(events)
                .await
                .map_err(|e| format!("deduplication failed: {}", e))?
        };

        if unique_events.is_empty() {
            return Ok(0);
        }

        let new_count = unique_events.len();
        let fingerprints: Vec<_> = unique_events.iter().map(|(_, fp)| fp.clone()).collect();
        let raw_events: Vec<_> = unique_events.into_iter().map(|(ev, _)| ev).collect();

        // Phase 2: Reconcile
        let reconciled = reconciliation.reconcile(raw_events);

        // Phase 3: Store
        storage
            .store_events(&reconciled)
            .await
            .map_err(|e| format!("storage failed: {}", e))?;

        // Phase 4: Mark fingerprints as seen
        {
            let mut dedup_guard = dedup.lock().await;
            dedup_guard
                .mark_seen(&fingerprints)
                .await
                .map_err(|e| format!("mark_seen failed: {}", e))?;
        }

    info!("Stored {} new events after deduplication", new_count);
    Ok(new_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_scheduler_defaults() {
        let scheduler = CollectionScheduler::new(30);
        assert_eq!(scheduler.interval_secs, 30);
        assert_eq!(scheduler.default_interval, 30);
        assert_eq!(scheduler.consecutive_errors, 0);
        assert!(!scheduler.is_running.load(Ordering::SeqCst));
    }

    #[test]
    fn test_backoff_interval_no_errors() {
        let scheduler = CollectionScheduler::new(30);
        // With 0 errors, backoff = 30 * 2^0 = 30
        assert_eq!(scheduler.backoff_interval(), 30);
    }

    #[test]
    fn test_backoff_interval_one_error() {
        let mut scheduler = CollectionScheduler::new(30);
        scheduler.consecutive_errors = 1;
        // 30 * 2^1 = 60
        assert_eq!(scheduler.backoff_interval(), 60);
    }

    #[test]
    fn test_backoff_interval_two_errors() {
        let mut scheduler = CollectionScheduler::new(30);
        scheduler.consecutive_errors = 2;
        // 30 * 2^2 = 120
        assert_eq!(scheduler.backoff_interval(), 120);
    }

    #[test]
    fn test_backoff_interval_three_errors() {
        let mut scheduler = CollectionScheduler::new(30);
        scheduler.consecutive_errors = 3;
        // 30 * 2^3 = 240
        assert_eq!(scheduler.backoff_interval(), 240);
    }

    #[test]
    fn test_backoff_interval_caps_at_max() {
        let mut scheduler = CollectionScheduler::new(30);
        scheduler.consecutive_errors = 4;
        // 30 * 2^4 = 480, capped at 300
        assert_eq!(scheduler.backoff_interval(), MAX_BACKOFF);
    }

    #[test]
    fn test_backoff_interval_large_error_count() {
        let mut scheduler = CollectionScheduler::new(30);
        scheduler.consecutive_errors = 100;
        // Should saturate and cap at MAX_BACKOFF
        assert_eq!(scheduler.backoff_interval(), MAX_BACKOFF);
    }

    #[test]
    fn test_calculate_interval_no_errors() {
        let scheduler = CollectionScheduler::new(30);
        assert_eq!(scheduler.calculate_interval(), 30);
    }

    #[test]
    fn test_calculate_interval_with_errors() {
        let mut scheduler = CollectionScheduler::new(30);
        scheduler.consecutive_errors = 2;
        assert_eq!(scheduler.calculate_interval(), 120);
    }

    #[test]
    fn test_calculate_interval_adaptive() {
        let mut scheduler = CollectionScheduler::new(30);
        scheduler.interval_secs = ADAPTIVE_INTERVAL;
        assert_eq!(scheduler.calculate_interval(), ADAPTIVE_INTERVAL);
    }

    #[test]
    fn test_stop_sets_flag() {
        let scheduler = CollectionScheduler::new(30);
        scheduler.is_running.store(true, Ordering::SeqCst);
        scheduler.stop();
        assert!(!scheduler.is_running.load(Ordering::SeqCst));
    }
}

/// Property-based tests for exponential backoff correctness.
// **Validates: Requirements 16.2, 16.3**
#[cfg(test)]
mod prop_tests_backoff {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::Config;

    proptest! {
        #![proptest_config(Config::with_cases(50))]

        /// Property 15.1: Backoff formula correctness
        /// The interval after N consecutive errors should be min(30 * 2^N, 300) seconds.
        // **Validates: Requirements 16.2**
        #[test]
        fn backoff_formula_correct(consecutive_errors in 0u32..20) {
            let mut scheduler = CollectionScheduler::new(30);
            scheduler.consecutive_errors = consecutive_errors;

            let result = scheduler.backoff_interval();
            let expected = (30u32.saturating_mul(2u32.saturating_pow(consecutive_errors))).min(MAX_BACKOFF);

            prop_assert_eq!(result, expected,
                "For {} consecutive errors: expected {}, got {}",
                consecutive_errors, expected, result);
        }

        /// Property 15.2: Max cap at 300
        /// The backoff interval never exceeds 300 seconds, regardless of error count.
        // **Validates: Requirements 16.2**
        #[test]
        fn backoff_never_exceeds_max(consecutive_errors in 0u32..1000) {
            let mut scheduler = CollectionScheduler::new(30);
            scheduler.consecutive_errors = consecutive_errors;

            let result = scheduler.backoff_interval();

            prop_assert!(result <= MAX_BACKOFF,
                "Backoff {} exceeded MAX_BACKOFF {} with {} consecutive errors",
                result, MAX_BACKOFF, consecutive_errors);
        }

        /// Property 15.3: Reset on success
        /// After a successful collection (consecutive_errors reset to 0), the interval
        /// should be either the default (30s) or adaptive (15s if >50 events).
        // **Validates: Requirements 16.3**
        #[test]
        fn reset_on_success_restores_default(
            prior_errors in 1u32..20,
            new_event_count in 0usize..200
        ) {
            let mut scheduler = CollectionScheduler::new(30);
            // Simulate errors accumulating
            scheduler.consecutive_errors = prior_errors;
            scheduler.interval_secs = scheduler.backoff_interval();

            // Simulate success: reset errors and set interval based on event count
            scheduler.consecutive_errors = 0;
            if new_event_count > ADAPTIVE_THRESHOLD {
                scheduler.interval_secs = ADAPTIVE_INTERVAL;
            } else {
                scheduler.interval_secs = scheduler.default_interval;
            }

            // After success, calculate_interval should return default or adaptive
            let result = scheduler.calculate_interval();
            if new_event_count > ADAPTIVE_THRESHOLD {
                prop_assert_eq!(result, ADAPTIVE_INTERVAL,
                    "Expected adaptive interval {} after {} events, got {}",
                    ADAPTIVE_INTERVAL, new_event_count, result);
            } else {
                prop_assert_eq!(result, 30,
                    "Expected default interval 30 after success, got {}", result);
            }
        }

        /// Property 15.4: Monotonic increase
        /// Each consecutive error increases the interval (until the cap is reached).
        // **Validates: Requirements 16.2**
        #[test]
        fn backoff_monotonically_increases(n in 0u32..10) {
            let mut scheduler_n = CollectionScheduler::new(30);
            scheduler_n.consecutive_errors = n;
            let interval_n = scheduler_n.backoff_interval();

            let mut scheduler_n1 = CollectionScheduler::new(30);
            scheduler_n1.consecutive_errors = n + 1;
            let interval_n1 = scheduler_n1.backoff_interval();

            prop_assert!(interval_n1 >= interval_n,
                "Interval should be monotonically increasing: {} (n={}) vs {} (n={})",
                interval_n, n, interval_n1, n + 1);
        }

        /// Property 15.5: Adaptive interval
        /// When >50 new events are collected on success, the interval should be 15s.
        // **Validates: Requirements 16.3**
        #[test]
        fn adaptive_interval_on_high_volume(event_count in 51usize..1000) {
            let mut scheduler = CollectionScheduler::new(30);

            // Simulate success with high event volume
            scheduler.consecutive_errors = 0;
            if event_count > ADAPTIVE_THRESHOLD {
                scheduler.interval_secs = ADAPTIVE_INTERVAL;
            } else {
                scheduler.interval_secs = scheduler.default_interval;
            }

            prop_assert_eq!(scheduler.interval_secs, ADAPTIVE_INTERVAL,
                "Expected adaptive interval {} for {} events, got {}",
                ADAPTIVE_INTERVAL, event_count, scheduler.interval_secs);
        }
    }
}
