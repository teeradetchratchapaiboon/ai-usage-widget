use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::CollectionError;
use crate::provider::{CollectionResult, ProviderAdapter, ProviderSummary, SourceMetadata};
use crate::types::{EventType, QuotaUsage, RawUsageEvent, TokenUsage};

/// Parsed structure for plan-usage-history.json (version 2 schema).
#[derive(Debug, Deserialize)]
struct PlanUsageHistory {
    version: u32,
    samples: Vec<UsageSample>,
}

/// A single usage sample from plan-usage-history.json.
#[derive(Debug, Deserialize)]
struct UsageSample {
    /// Timestamp in milliseconds since epoch.
    t: i64,
    /// Organization UUID (absent in some samples).
    #[serde(default)]
    org: Option<String>,
    /// Usage percentages.
    u: UsagePercentages,
}

/// Usage percentage breakdown within a sample.
///
/// Every field is optional: real samples omit dimensions that do not apply to
/// the plan (`xu` is absent on plans without excess usage). Requiring them made
/// serde reject the whole file, which silently dropped Claude's quota data.
#[derive(Debug, Deserialize)]
struct UsagePercentages {
    /// Fast hours usage percentage.
    #[serde(default)]
    fh: Option<f64>,
    /// Standard usage percentage.
    #[serde(default)]
    sd: Option<f64>,
    /// Excess usage percentage.
    #[serde(default)]
    xu: Option<f64>,
}

/// Parsed structure for buddy-tokens.json.
#[derive(Debug, Deserialize)]
struct BuddyTokens {
    #[serde(rename = "tokens-today")]
    tokens_today: TokensToday,
}

/// Daily token count from buddy-tokens.json.
#[derive(Debug, Deserialize)]
struct TokensToday {
    date: String,
    tokens: u64,
}

/// Adapter for collecting usage data from Claude Desktop.
///
/// Reads plan-usage-history.json and buddy-tokens.json from the Claude
/// Desktop Windows Store sandboxed data directory.
pub struct ClaudeAdapter {
    /// Path to the Claude Desktop data directory.
    data_dir: PathBuf,
    /// Timestamp (ms since epoch) of the last processed sample.
    last_sample_time: Mutex<Option<i64>>,
}

impl ClaudeAdapter {
    /// Create a new ClaudeAdapter with the given data directory.
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            last_sample_time: Mutex::new(None),
        }
    }

    /// Hash an org_id with SHA-256 and return the hex string.
    fn hash_org_id(org_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(org_id.as_bytes());
        let result = hasher.finalize();
        result.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Convert a millisecond timestamp to a DateTime<Utc>.
    /// Returns None if the timestamp is invalid.
    fn ms_to_datetime(ms: i64) -> Option<DateTime<Utc>> {
        let secs = ms / 1000;
        let nsecs = ((ms % 1000) * 1_000_000) as u32;
        Utc.timestamp_opt(secs, nsecs).single()
    }

    /// Work out when each quota window rolls over.
    ///
    /// Claude publishes percentages only — no reset timestamps — so the times
    /// are read off the history instead: a counter that falls sharply is a
    /// window that just rolled over, and the next one lands a window-length
    /// later. A reset already in the past means the drop is too old to place
    /// the current window, so nothing is claimed.
    fn infer_quota_resets(samples: &[UsageSample]) -> crate::provider::QuotaResets {
        crate::provider::QuotaResets {
            fast_hours: Self::infer_reset(samples, |s| s.u.fh, Duration::hours(5)),
            weekly: Self::infer_reset(samples, |s| s.u.sd, Duration::days(7)),
            // Every time here is projected off a drop, never a stated value
            estimated: true,
        }
    }

    /// When the percentages in `sample` were recorded, per window.
    ///
    /// Claude stamps every sample with `t`, so the age of a reading is stated
    /// by the file rather than inferred — unlike its reset times, which are
    /// reconstructed. A window the sample does not report gets no timestamp:
    /// there is no reading to be fresh or stale.
    fn observed_from(sample: &UsageSample) -> crate::provider::QuotaObservations {
        let observed_at = Self::ms_to_datetime(sample.t);
        crate::provider::QuotaObservations {
            fast_hours: sample.u.fh.and(observed_at),
            weekly: sample.u.sd.and(observed_at),
        }
    }

    /// Observation times taken from the newest sample on disk.
    fn read_quota_observed(&self) -> crate::provider::QuotaObservations {
        let plan_path = self.data_dir.join("plan-usage-history.json");
        let Ok(content) = std::fs::read_to_string(&plan_path) else {
            return crate::provider::QuotaObservations::default();
        };
        match serde_json::from_str::<PlanUsageHistory>(&content) {
            Ok(history) if history.version == 2 => history
                .samples
                .last()
                .map(Self::observed_from)
                .unwrap_or_default(),
            _ => crate::provider::QuotaObservations::default(),
        }
    }

    /// Percentage-point fall that counts as a window rolling over rather than
    /// as the ordinary jitter of a rounded percentage.
    const RESET_DROP_THRESHOLD: f64 = 10.0;

    /// Find the most recent rollover of one counter and project it forward.
    fn infer_reset<F>(samples: &[UsageSample], value: F, window: Duration) -> Option<DateTime<Utc>>
    where
        F: Fn(&UsageSample) -> Option<f64>,
    {
        let started_at =
            samples
                .windows(2)
                .rev()
                .find_map(|pair| match (value(&pair[0]), value(&pair[1])) {
                    (Some(before), Some(after)) if before - after >= Self::RESET_DROP_THRESHOLD => {
                        Some(pair[1].t)
                    }
                    _ => None,
                })?;

        let resets_at = Self::ms_to_datetime(started_at)? + window;
        (resets_at > Utc::now()).then_some(resets_at)
    }

    /// Parse plan-usage-history.json and return usage events newer than the checkpoint.
    fn parse_plan_usage_history(
        &self,
        since_ms: Option<i64>,
    ) -> Result<(Vec<RawUsageEvent>, u64, Option<i64>), CollectionError> {
        let path = self.data_dir.join("plan-usage-history.json");
        let content = std::fs::read_to_string(&path)?;
        let bytes_read = content.len() as u64;

        let history: PlanUsageHistory = serde_json::from_str(&content).map_err(|e| {
            CollectionError::ParseError(crate::error::ParseError::MalformedJson(e.to_string()))
        })?;

        if history.version != 2 {
            return Err(CollectionError::ParseError(
                crate::error::ParseError::UnexpectedSchema(format!(
                    "expected version 2, got {}",
                    history.version
                )),
            ));
        }

        let mut events = Vec::new();
        let mut max_time: Option<i64> = None;

        for sample in &history.samples {
            // Only process samples newer than the checkpoint
            if let Some(checkpoint) = since_ms {
                if sample.t <= checkpoint {
                    continue;
                }
            }

            let timestamp = Self::ms_to_datetime(sample.t).ok_or_else(|| {
                CollectionError::ParseError(crate::error::ParseError::InvalidTimestamp(
                    sample.t.to_string(),
                    "invalid millisecond timestamp".to_string(),
                ))
            })?;

            let session_hash = Self::hash_org_id(sample.org.as_deref().unwrap_or("unknown"));

            let event = RawUsageEvent {
                provider_id: "claude".to_string(),
                event_type: EventType::QuotaSample,
                timestamp,
                model: None,
                tokens: TokenUsage::default(),
                context_window: None,
                quota: Some(QuotaUsage {
                    fast_hours_pct: sample.u.fh,
                    standard_pct: sample.u.sd,
                    excess_pct: sample.u.xu,
                    daily_tokens: None,
                }),
                session_hash: Some(session_hash),
                project_hash: None,
                source_file: Some("plan-usage-history.json".to_string()),
                raw_metadata: None,
            };

            events.push(event);

            // Track the maximum timestamp seen
            match max_time {
                Some(current_max) if sample.t > current_max => max_time = Some(sample.t),
                None => max_time = Some(sample.t),
                _ => {}
            }
        }

        Ok((events, bytes_read, max_time))
    }

    /// Parse buddy-tokens.json and return a daily aggregate event.
    fn parse_buddy_tokens(&self) -> Result<(Option<RawUsageEvent>, u64), CollectionError> {
        let path = self.data_dir.join("buddy-tokens.json");
        let content = std::fs::read_to_string(&path)?;
        let bytes_read = content.len() as u64;

        let buddy: BuddyTokens = serde_json::from_str(&content).map_err(|e| {
            CollectionError::ParseError(crate::error::ParseError::MalformedJson(e.to_string()))
        })?;

        // Parse the date string to create a timestamp (start of day UTC)
        let timestamp = chrono::NaiveDate::parse_from_str(&buddy.tokens_today.date, "%Y-%m-%d")
            .map_err(|e| {
                CollectionError::ParseError(crate::error::ParseError::InvalidTimestamp(
                    buddy.tokens_today.date.clone(),
                    e.to_string(),
                ))
            })?
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| {
                CollectionError::ParseError(crate::error::ParseError::InvalidTimestamp(
                    buddy.tokens_today.date.clone(),
                    "failed to create datetime".to_string(),
                ))
            })?;

        let timestamp = timestamp.and_utc();

        let event = RawUsageEvent {
            provider_id: "claude".to_string(),
            event_type: EventType::DailyAggregate,
            timestamp,
            model: None,
            tokens: TokenUsage {
                total_tokens: Some(buddy.tokens_today.tokens),
                ..TokenUsage::default()
            },
            context_window: None,
            quota: None,
            session_hash: None,
            project_hash: None,
            source_file: Some("buddy-tokens.json".to_string()),
            raw_metadata: None,
        };

        Ok((Some(event), bytes_read))
    }
}

impl ProviderAdapter for ClaudeAdapter {
    fn provider_id(&self) -> &str {
        "claude"
    }

    fn display_name(&self) -> &str {
        "Claude Desktop"
    }

    fn is_available(&self) -> bool {
        self.data_dir.exists() && self.data_dir.is_dir()
    }

    fn collect(&self, since: Option<DateTime<Utc>>) -> Result<CollectionResult, CollectionError> {
        if !self.is_available() {
            log::error!(
                "Claude data directory is inaccessible: {}",
                self.data_dir.display()
            );
            return Err(CollectionError::DataSourceUnavailable(
                "claude".to_string(),
                format!("data directory not accessible: {}", self.data_dir.display()),
            ));
        }

        let mut all_events = Vec::new();
        let mut total_bytes: u64 = 0;
        let mut files_read: u32 = 0;
        let mut errors: Vec<String> = Vec::new();
        let mut checkpoint = since.unwrap_or_else(Utc::now);

        // Determine the checkpoint in milliseconds
        let last_sample = self.last_sample_time.lock().unwrap();
        let since_ms = if let Some(dt) = since {
            Some(dt.timestamp_millis())
        } else {
            *last_sample
        };
        drop(last_sample);

        // Parse plan-usage-history.json
        match self.parse_plan_usage_history(since_ms) {
            Ok((events, bytes, max_time)) => {
                files_read += 1;
                total_bytes += bytes;

                if !events.is_empty() {
                    // Update checkpoint to the latest event timestamp
                    if let Some(last) = events.last() {
                        if last.timestamp > checkpoint {
                            checkpoint = last.timestamp;
                        }
                    }
                    all_events.extend(events);
                }

                // Update the last sample time
                if let Some(mt) = max_time {
                    let mut last = self.last_sample_time.lock().unwrap();
                    *last = Some(mt);
                }
            }
            Err(e) => {
                errors.push(format!("plan-usage-history.json: {}", e));
            }
        }

        // Parse buddy-tokens.json
        match self.parse_buddy_tokens() {
            Ok((event, bytes)) => {
                files_read += 1;
                total_bytes += bytes;

                if let Some(evt) = event {
                    if evt.timestamp > checkpoint {
                        checkpoint = evt.timestamp;
                    }
                    all_events.push(evt);
                }
            }
            Err(e) => {
                errors.push(format!("buddy-tokens.json: {}", e));
            }
        }

        Ok(CollectionResult {
            events: all_events,
            checkpoint,
            source_metadata: SourceMetadata {
                files_read,
                bytes_processed: total_bytes,
                errors,
            },
        })
    }

    fn get_current_summary(&self) -> Result<ProviderSummary, CollectionError> {
        let mut quota = None;
        let mut tokens_today = None;
        let mut quota_resets = crate::provider::QuotaResets::default();
        let mut quota_observed = crate::provider::QuotaObservations::default();

        // Try to read current quota from plan-usage-history.json
        let plan_path = self.data_dir.join("plan-usage-history.json");
        if plan_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&plan_path) {
                if let Ok(history) = serde_json::from_str::<PlanUsageHistory>(&content) {
                    if history.version == 2 {
                        if let Some(latest) = history.samples.last() {
                            quota = Some(QuotaUsage {
                                fast_hours_pct: latest.u.fh,
                                standard_pct: latest.u.sd,
                                excess_pct: latest.u.xu,
                                daily_tokens: None,
                            });
                            quota_observed = Self::observed_from(latest);
                        }
                        quota_resets = Self::infer_quota_resets(&history.samples);
                    }
                }
            }
        }

        // Try to read today's tokens from buddy-tokens.json
        let buddy_path = self.data_dir.join("buddy-tokens.json");
        if buddy_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&buddy_path) {
                if let Ok(buddy) = serde_json::from_str::<BuddyTokens>(&content) {
                    tokens_today = Some(buddy.tokens_today.tokens);
                }
            }
        }

        Ok(ProviderSummary {
            provider_id: "claude".to_string(),
            display_name: "Claude Desktop".to_string(),
            is_available: self.is_available(),
            current_model: None,
            tokens_today,
            quota,
            context_window: None,
            last_activity: None,
            quota_resets,
            quota_observed,
        })
    }

    fn quota_resets(&self) -> crate::provider::QuotaResets {
        let plan_path = self.data_dir.join("plan-usage-history.json");
        let Ok(content) = std::fs::read_to_string(&plan_path) else {
            return crate::provider::QuotaResets::default();
        };
        match serde_json::from_str::<PlanUsageHistory>(&content) {
            Ok(history) if history.version == 2 => Self::infer_quota_resets(&history.samples),
            _ => crate::provider::QuotaResets::default(),
        }
    }

    fn quota_observed(&self) -> crate::provider::QuotaObservations {
        self.read_quota_observed()
    }

    fn last_checkpoint(&self) -> Option<DateTime<Utc>> {
        let last = self.last_sample_time.lock().unwrap();
        last.and_then(Self::ms_to_datetime)
    }
}

#[cfg(test)]
mod prop_tests_claude_checkpoint {
    use super::*;
    use proptest::prelude::*;
    use std::fs;
    use tempfile::TempDir;

    // **Validates: Requirements 14.4**

    // ─── Strategies ─────────────────────────────────────────────────────────────

    /// Generate a sorted vector of timestamps (milliseconds since epoch)
    /// in a realistic range (2023-01-01 to 2025-12-31).
    fn arb_sorted_timestamps(max_len: usize) -> impl Strategy<Value = Vec<i64>> {
        let min_ms: i64 = 1_672_531_200_000; // 2023-01-01T00:00:00Z
        let max_ms: i64 = 1_767_225_600_000; // 2025-12-31T00:00:00Z

        proptest::collection::vec(min_ms..max_ms, 0..max_len).prop_map(|mut v| {
            v.sort();
            v.dedup();
            v
        })
    }

    /// Generate a checkpoint value in a plausible range.
    fn arb_checkpoint_ms() -> impl Strategy<Value = i64> {
        let min_ms: i64 = 1_672_531_200_000;
        let max_ms: i64 = 1_767_225_600_000;
        min_ms..max_ms
    }

    /// Build plan-usage-history.json content from a list of timestamps.
    fn build_plan_json(timestamps: &[i64]) -> String {
        let samples: Vec<String> = timestamps
            .iter()
            .enumerate()
            .map(|(i, t)| {
                format!(
                    r#"{{"t":{},"org":"org-{}","u":{{"fh":{},"sd":{},"xu":{}}}}}"#,
                    t,
                    i % 5,
                    (i as f64) * 1.5,
                    (i as f64) * 0.8,
                    (i as f64) * 0.2
                )
            })
            .collect();
        format!(r#"{{"version":2,"samples":[{}]}}"#, samples.join(","))
    }

    /// Write the JSON to a temp dir and create a ClaudeAdapter for it.
    fn setup_adapter(dir: &TempDir, timestamps: &[i64]) -> ClaudeAdapter {
        let json = build_plan_json(timestamps);
        fs::write(dir.path().join("plan-usage-history.json"), &json).unwrap();
        ClaudeAdapter::new(dir.path().to_path_buf())
    }

    // ─── Property Tests ─────────────────────────────────────────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(20))]

        /// Property 1: Only newer samples pass — when a checkpoint is provided,
        /// only samples with t > checkpoint_ms appear in the output.
        #[test]
        fn prop_only_newer_samples_pass(
            timestamps in arb_sorted_timestamps(20),
            checkpoint in arb_checkpoint_ms()
        ) {
            let dir = tempfile::tempdir().unwrap();
            let adapter = setup_adapter(&dir, &timestamps);

            let (events, _, _) = adapter.parse_plan_usage_history(Some(checkpoint)).unwrap();

            for event in &events {
                let event_ms = event.timestamp.timestamp_millis();
                prop_assert!(
                    event_ms > checkpoint,
                    "Event with timestamp {} should be > checkpoint {}",
                    event_ms,
                    checkpoint
                );
            }
        }

        /// Property 2: No checkpoint returns all — when since_ms is None,
        /// all samples in the file are returned.
        #[test]
        fn prop_no_checkpoint_returns_all(timestamps in arb_sorted_timestamps(20)) {
            let dir = tempfile::tempdir().unwrap();
            let adapter = setup_adapter(&dir, &timestamps);

            let (events, _, _) = adapter.parse_plan_usage_history(None).unwrap();

            prop_assert_eq!(
                events.len(),
                timestamps.len(),
                "Without checkpoint, should return all {} samples but got {}",
                timestamps.len(),
                events.len()
            );
        }

        /// Property 3: Checkpoint at max returns empty — when checkpoint equals
        /// the maximum sample timestamp, no samples pass through.
        #[test]
        fn prop_checkpoint_at_max_returns_empty(
            timestamps in arb_sorted_timestamps(20).prop_filter(
                "need at least one timestamp",
                |ts| !ts.is_empty()
            )
        ) {
            let dir = tempfile::tempdir().unwrap();
            let adapter = setup_adapter(&dir, &timestamps);
            let max_ts = *timestamps.iter().max().unwrap();

            let (events, _, _) = adapter.parse_plan_usage_history(Some(max_ts)).unwrap();

            prop_assert!(
                events.is_empty(),
                "Checkpoint at max timestamp {} should yield 0 events, got {}",
                max_ts,
                events.len()
            );
        }

        /// Property 4: Monotonic filtering — if we filter with an earlier checkpoint,
        /// we always get at least as many results as filtering with a later checkpoint.
        #[test]
        fn prop_monotonic_filtering(
            timestamps in arb_sorted_timestamps(20),
            cp1 in arb_checkpoint_ms(),
            cp2 in arb_checkpoint_ms()
        ) {
            let dir = tempfile::tempdir().unwrap();
            let adapter = setup_adapter(&dir, &timestamps);

            let earlier = cp1.min(cp2);
            let later = cp1.max(cp2);

            let (events_earlier, _, _) = adapter.parse_plan_usage_history(Some(earlier)).unwrap();
            let (events_later, _, _) = adapter.parse_plan_usage_history(Some(later)).unwrap();

            prop_assert!(
                events_earlier.len() >= events_later.len(),
                "Earlier checkpoint {} yielded {} events, later checkpoint {} yielded {} events (monotonicity violated)",
                earlier,
                events_earlier.len(),
                later,
                events_later.len()
            );
        }

        /// Property 5: Correct event count — the number of events returned equals
        /// exactly the count of samples where t > checkpoint.
        #[test]
        fn prop_correct_event_count(
            timestamps in arb_sorted_timestamps(20),
            checkpoint in arb_checkpoint_ms()
        ) {
            let dir = tempfile::tempdir().unwrap();
            let adapter = setup_adapter(&dir, &timestamps);

            let expected_count = timestamps.iter().filter(|&&t| t > checkpoint).count();
            let (events, _, _) = adapter.parse_plan_usage_history(Some(checkpoint)).unwrap();

            prop_assert_eq!(
                events.len(),
                expected_count,
                "Expected {} events with t > {}, got {}",
                expected_count,
                checkpoint,
                events.len()
            );
        }
    }
}

#[cfg(test)]
#[path = "claude_quota_tests.rs"]
mod quota_observation_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    /// Build a plan-usage-history.json from (minutes-ago, fh, sd) triples.
    fn history_json(samples: &[(i64, f64, f64)]) -> String {
        let now_ms = Utc::now().timestamp_millis();
        let entries: Vec<String> = samples
            .iter()
            .map(|(mins_ago, fh, sd)| {
                format!(
                    r#"{{"t":{},"org":"abc","u":{{"fh":{},"sd":{}}}}}"#,
                    now_ms - mins_ago * 60_000,
                    fh,
                    sd
                )
            })
            .collect();
        format!(r#"{{"version":2,"samples":[{}]}}"#, entries.join(","))
    }

    #[test]
    fn test_infers_reset_times_from_a_counter_dropping() {
        // Claude publishes percentages only, so the five-hour window is placed
        // by the last time the counter fell: it restarted 60 minutes ago, so
        // it rolls over again in about four hours.
        let dir = tempfile::tempdir().unwrap();
        let json = history_json(&[(180, 40.0, 50.0), (60, 3.0, 51.0), (5, 22.0, 52.0)]);
        fs::write(dir.path().join("plan-usage-history.json"), json).unwrap();

        let resets = ClaudeAdapter::new(dir.path().to_path_buf()).quota_resets();
        let minutes_out = (resets.fast_hours.expect("five-hour reset") - Utc::now()).num_minutes();
        assert!(
            (235..=245).contains(&minutes_out),
            "expected ~240 minutes out, got {}",
            minutes_out
        );
        // `sd` only ever climbed here, so no weekly rollover can be placed
        assert_eq!(resets.weekly, None);

        // Reconstructed, not published — the UI has to be able to say so
        assert!(resets.estimated);
    }

    #[test]
    fn test_reset_older_than_its_window_is_not_reported() {
        // The drop is six hours old: the window it started has already rolled
        // over again, so its reset time would be a lie.
        let dir = tempfile::tempdir().unwrap();
        let json = history_json(&[(400, 90.0, 10.0), (360, 1.0, 11.0)]);
        fs::write(dir.path().join("plan-usage-history.json"), json).unwrap();

        assert_eq!(
            ClaudeAdapter::new(dir.path().to_path_buf())
                .quota_resets()
                .fast_hours,
            None
        );
    }

    #[test]
    fn test_parses_samples_without_excess_field() {
        // Real plan-usage-history.json samples carry only fh/sd; requiring xu
        // made serde reject the file and silently dropped all quota data.
        let dir = tempfile::tempdir().unwrap();
        let json =
            r#"{"version":2,"samples":[{"t":1785515157692,"org":"abc","u":{"fh":60,"sd":82}}]}"#;
        fs::write(dir.path().join("plan-usage-history.json"), json).unwrap();

        let adapter = ClaudeAdapter::new(dir.path().to_path_buf());
        let summary = adapter.get_current_summary().unwrap();
        let quota = summary.quota.expect("quota should be parsed");

        assert_eq!(quota.fast_hours_pct, Some(60.0));
        assert_eq!(quota.standard_pct, Some(82.0));
        assert_eq!(quota.excess_pct, None);
    }

    #[test]
    fn test_hash_org_id() {
        let hash = ClaudeAdapter::hash_org_id("test-org-uuid");
        assert_eq!(hash.len(), 64); // SHA-256 produces 64 hex chars
                                    // Same input should produce same hash
        assert_eq!(hash, ClaudeAdapter::hash_org_id("test-org-uuid"));
        // Different input should produce different hash
        assert_ne!(hash, ClaudeAdapter::hash_org_id("other-org-uuid"));
    }

    #[test]
    fn test_ms_to_datetime() {
        let dt = ClaudeAdapter::ms_to_datetime(1700000000000).unwrap();
        assert_eq!(dt.timestamp(), 1700000000);

        let dt = ClaudeAdapter::ms_to_datetime(1700000000500).unwrap();
        assert_eq!(dt.timestamp_millis(), 1700000000500);
    }

    #[test]
    fn test_is_available_missing_dir() {
        let adapter = ClaudeAdapter::new(PathBuf::from("/nonexistent/path"));
        assert!(!adapter.is_available());
    }

    #[test]
    fn test_is_available_existing_dir() {
        let dir = create_test_dir();
        let adapter = ClaudeAdapter::new(dir.path().to_path_buf());
        assert!(adapter.is_available());
    }

    #[test]
    fn test_parse_plan_usage_history() {
        let dir = create_test_dir();
        let json = r#"{"version":2,"samples":[{"t":1700000000000,"org":"abc-123","u":{"fh":45.5,"sd":30.0,"xu":5.0}},{"t":1700000060000,"org":"abc-123","u":{"fh":50.0,"sd":32.0,"xu":6.0}}]}"#;
        fs::write(dir.path().join("plan-usage-history.json"), json).unwrap();

        let adapter = ClaudeAdapter::new(dir.path().to_path_buf());
        let (events, bytes, max_time) = adapter.parse_plan_usage_history(None).unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(bytes, json.len() as u64);
        assert_eq!(max_time, Some(1700000060000));

        // Check first event
        assert_eq!(events[0].event_type, EventType::QuotaSample);
        assert_eq!(events[0].provider_id, "claude");
        let quota = events[0].quota.as_ref().unwrap();
        assert_eq!(quota.fast_hours_pct, Some(45.5));
        assert_eq!(quota.standard_pct, Some(30.0));
        assert_eq!(quota.excess_pct, Some(5.0));
        assert!(events[0].session_hash.is_some());
    }

    #[test]
    fn test_parse_plan_usage_history_filters_by_checkpoint() {
        let dir = create_test_dir();
        let json = r#"{"version":2,"samples":[{"t":1700000000000,"org":"abc-123","u":{"fh":45.5,"sd":30.0,"xu":5.0}},{"t":1700000060000,"org":"abc-123","u":{"fh":50.0,"sd":32.0,"xu":6.0}}]}"#;
        fs::write(dir.path().join("plan-usage-history.json"), json).unwrap();

        let adapter = ClaudeAdapter::new(dir.path().to_path_buf());
        // Only get samples after the first one
        let (events, _, _) = adapter
            .parse_plan_usage_history(Some(1700000000000))
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].timestamp.timestamp(), 1700000060);
    }

    #[test]
    fn test_parse_plan_usage_history_wrong_version() {
        let dir = create_test_dir();
        let json = r#"{"version":1,"samples":[]}"#;
        fs::write(dir.path().join("plan-usage-history.json"), json).unwrap();

        let adapter = ClaudeAdapter::new(dir.path().to_path_buf());
        let result = adapter.parse_plan_usage_history(None);

        assert!(result.is_err());
    }

    #[test]
    fn test_parse_buddy_tokens() {
        let dir = create_test_dir();
        let json = r#"{"tokens-today":{"date":"2026-07-30","tokens":368886}}"#;
        fs::write(dir.path().join("buddy-tokens.json"), json).unwrap();

        let adapter = ClaudeAdapter::new(dir.path().to_path_buf());
        let (event, bytes) = adapter.parse_buddy_tokens().unwrap();

        assert_eq!(bytes, json.len() as u64);
        let event = event.unwrap();
        assert_eq!(event.event_type, EventType::DailyAggregate);
        assert_eq!(event.tokens.total_tokens, Some(368886));
        assert_eq!(event.source_file.as_deref(), Some("buddy-tokens.json"));
    }

    #[test]
    fn test_collect_unavailable_directory() {
        let adapter = ClaudeAdapter::new(PathBuf::from("/nonexistent/claude/path"));
        let result = adapter.collect(None);
        assert!(result.is_err());
        match result.unwrap_err() {
            CollectionError::DataSourceUnavailable(provider, _) => {
                assert_eq!(provider, "claude");
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }

    #[test]
    fn test_collect_full() {
        let dir = create_test_dir();
        let plan_json = r#"{"version":2,"samples":[{"t":1700000000000,"org":"org-1","u":{"fh":45.5,"sd":30.0,"xu":5.0}}]}"#;
        let buddy_json = r#"{"tokens-today":{"date":"2026-07-30","tokens":100000}}"#;
        fs::write(dir.path().join("plan-usage-history.json"), plan_json).unwrap();
        fs::write(dir.path().join("buddy-tokens.json"), buddy_json).unwrap();

        let adapter = ClaudeAdapter::new(dir.path().to_path_buf());
        let result = adapter.collect(None).unwrap();

        assert_eq!(result.events.len(), 2);
        assert_eq!(result.source_metadata.files_read, 2);
        assert_eq!(result.source_metadata.errors.len(), 0);
    }

    #[test]
    fn test_last_checkpoint_initially_none() {
        let adapter = ClaudeAdapter::new(PathBuf::from("/tmp/test"));
        assert!(adapter.last_checkpoint().is_none());
    }
}
