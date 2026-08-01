//! Integration tests for the full data pipeline:
//! file read → parse → dedup → reconcile → store → query.
//!
//! **Validates: Requirements 1.1, 1.3, 1.7, 13.2, 14.1**
//!
//! These tests use fixture data mimicking real Codex and Claude session logs,
//! then exercise the complete pipeline from collection through storage and query.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use chrono::{Duration, Utc};
use tempfile::TempDir;

use crate::config::CodexConfig;
use crate::dedup::DeduplicationEngine;
use crate::provider::ProviderAdapter;
use crate::providers::claude::ClaudeAdapter;
use crate::providers::codex::CodexAdapter;
use crate::query_types::{Granularity, TimeRange};
use crate::reconcile::ReconciliationEngine;
use crate::storage::StorageLayer;

// ─── Fixture Data ───────────────────────────────────────────────────────────

/// Realistic Codex JSONL session log fixture (mimics ~/.codex/sessions/2024/07/01/session1.jsonl).
const CODEX_FIXTURE_JSONL: &str = r#"{"timestamp":"2024-07-01T10:00:00Z","type":"session_meta","payload":{"type":"session_meta","model":"gpt-4o"}}
{"timestamp":"2024-07-01T10:00:05Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":150,"output_tokens":300,"total_tokens":450},"model_context_window":128000}}}
{"timestamp":"2024-07-01T10:00:15Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200,"output_tokens":400,"total_tokens":600},"model_context_window":128000}}}
{"timestamp":"2024-07-01T10:01:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":500,"output_tokens":1000,"total_tokens":1500},"model_context_window":128000}}}
"#;

/// Second Codex session with a different model.
const CODEX_FIXTURE_JSONL_2: &str = r#"{"timestamp":"2024-07-01T11:00:00Z","type":"session_meta","payload":{"type":"session_meta","model":"o1-mini"}}
{"timestamp":"2024-07-01T11:00:10Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":80,"output_tokens":160,"total_tokens":240},"model_context_window":128000}}}
{"timestamp":"2024-07-01T11:00:30Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":120,"output_tokens":250,"total_tokens":370},"model_context_window":128000}}}
"#;

/// Claude Desktop plan-usage-history.json fixture (version 2 schema).
const CLAUDE_PLAN_USAGE_FIXTURE: &str = r#"{"version":2,"samples":[{"t":1719828000000,"org":"org-test-123","u":{"fh":45.5,"sd":30.0,"xu":5.0}},{"t":1719828060000,"org":"org-test-123","u":{"fh":50.0,"sd":32.5,"xu":6.0}},{"t":1719828120000,"org":"org-test-456","u":{"fh":20.0,"sd":15.0,"xu":2.0}}]}"#;

/// Claude Desktop buddy-tokens.json fixture.
const CLAUDE_BUDDY_TOKENS_FIXTURE: &str =
    r#"{"tokens-today":{"date":"2024-07-01","tokens":75000}}"#;

// ─── Helper Functions ───────────────────────────────────────────────────────

/// Create the Codex sessions directory structure and write fixture JSONL files.
fn setup_codex_fixtures(base_dir: &Path) {
    // Create sessions/2024/07/01/ directory structure
    let session_dir = base_dir.join("sessions").join("2024").join("07").join("01");
    fs::create_dir_all(&session_dir).unwrap();

    // Write first session JSONL
    fs::write(session_dir.join("session1.jsonl"), CODEX_FIXTURE_JSONL).unwrap();

    // Write second session JSONL
    fs::write(session_dir.join("session2.jsonl"), CODEX_FIXTURE_JSONL_2).unwrap();
}

/// Create the Claude data directory and write fixture JSON files.
fn setup_claude_fixtures(base_dir: &Path) {
    fs::create_dir_all(base_dir).unwrap();
    fs::write(
        base_dir.join("plan-usage-history.json"),
        CLAUDE_PLAN_USAGE_FIXTURE,
    )
    .unwrap();
    fs::write(
        base_dir.join("buddy-tokens.json"),
        CLAUDE_BUDDY_TOKENS_FIXTURE,
    )
    .unwrap();
}

// ─── Integration Tests ──────────────────────────────────────────────────────

/// Test the full pipeline: Codex file read → parse → dedup → reconcile → store → query.
#[test]
fn test_full_pipeline_codex_only() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let tmp = TempDir::new().unwrap();
        let codex_dir = tmp.path().join("codex");

        // Setup fixtures
        setup_codex_fixtures(&codex_dir);

        // 1. Create CodexAdapter and collect events
        let config = CodexConfig {
            sessions_dir: codex_dir.join("sessions"),
            state_db_path: codex_dir.join("nonexistent.sqlite"),
            enabled: true,
        };
        let codex_adapter = CodexAdapter::new(&config);
        assert!(codex_adapter.is_available());

        let result = codex_adapter.collect(None).unwrap();
        // 3 events from session1 + 2 events from session2 = 5 total token_count events
        assert_eq!(
            result.events.len(),
            5,
            "Should collect 5 token_count events from Codex JSONL"
        );

        // 2. Deduplicate events
        let db_path = tmp.path().join("test.db");
        let storage = StorageLayer::new(&db_path).await.unwrap();
        let pool = Arc::new(storage.pool().clone());
        let mut dedup = DeduplicationEngine::new(pool).await.unwrap();

        let deduped = dedup.deduplicate(result.events).await.unwrap();
        assert_eq!(
            deduped.len(),
            5,
            "First run should pass all events through dedup"
        );

        // 3. Reconcile events
        let reconciliation = ReconciliationEngine::new();
        let raw_events: Vec<_> = deduped.iter().map(|(e, _)| e.clone()).collect();
        let fingerprints: Vec<_> = deduped.iter().map(|(_, fp)| fp.clone()).collect();
        let reconciled = reconciliation.reconcile(raw_events);

        // Events have different timestamps (>1s apart) and different models,
        // so they should remain as separate reconciled events.
        assert_eq!(
            reconciled.len(),
            5,
            "All events should be separate after reconciliation"
        );

        // 4. Store reconciled events
        let stored_count = storage.store_events(&reconciled).await.unwrap();
        assert_eq!(stored_count, 5, "Should store 5 events");

        // 5. Mark fingerprints as seen
        dedup.mark_seen(&fingerprints).await.unwrap();

        // 6. Query and verify aggregation
        let range = TimeRange {
            start: "2024-07-01T00:00:00Z".parse().unwrap(),
            end: "2024-07-02T00:00:00Z".parse().unwrap(),
        };
        let records = storage
            .get_history(&range, Granularity::Daily)
            .await
            .unwrap();

        // Should have a single daily bucket for "codex" provider
        assert!(
            !records.is_empty(),
            "Should have at least one aggregated record"
        );
        let codex_record = records.iter().find(|r| r.provider_id == "codex").unwrap();

        // Total tokens: 450 + 600 + 1500 + 240 + 370 = 3160
        assert_eq!(codex_record.total_tokens, Some(3160));
        assert_eq!(codex_record.input_tokens, Some(150 + 200 + 500 + 80 + 120));
        assert_eq!(
            codex_record.output_tokens,
            Some(300 + 400 + 1000 + 160 + 250)
        );

        // 7. Verify dedup rejects same events on second collection
        let result2 = codex_adapter.collect(None).unwrap();
        // CodexAdapter uses incremental reads, so second collect returns 0 new events
        assert_eq!(
            result2.events.len(),
            0,
            "Incremental read should return 0 events on re-read"
        );

        storage.close().await;
    });
}

/// Test the full pipeline: Claude file read → parse → dedup → reconcile → store → query.
#[test]
fn test_full_pipeline_claude_only() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let tmp = TempDir::new().unwrap();
        let claude_dir = tmp.path().join("claude");

        // Setup fixtures
        setup_claude_fixtures(&claude_dir);

        // 1. Create ClaudeAdapter and collect events
        let claude_adapter = ClaudeAdapter::new(claude_dir.clone());
        assert!(claude_adapter.is_available());

        let result = claude_adapter.collect(None).unwrap();
        // 3 quota samples from plan-usage-history + 1 daily aggregate from buddy-tokens = 4
        assert_eq!(
            result.events.len(),
            4,
            "Should collect 4 events from Claude fixtures"
        );

        // Verify event types
        let quota_events: Vec<_> = result
            .events
            .iter()
            .filter(|e| e.event_type == crate::types::EventType::QuotaSample)
            .collect();
        let daily_events: Vec<_> = result
            .events
            .iter()
            .filter(|e| e.event_type == crate::types::EventType::DailyAggregate)
            .collect();
        assert_eq!(quota_events.len(), 3, "Should have 3 QuotaSample events");
        assert_eq!(daily_events.len(), 1, "Should have 1 DailyAggregate event");

        // Verify buddy-tokens daily aggregate has correct total
        assert_eq!(daily_events[0].tokens.total_tokens, Some(75000));

        // 2. Deduplicate
        let db_path = tmp.path().join("test_claude.db");
        let storage = StorageLayer::new(&db_path).await.unwrap();
        let pool = Arc::new(storage.pool().clone());
        let mut dedup = DeduplicationEngine::new(pool).await.unwrap();

        let deduped = dedup.deduplicate(result.events).await.unwrap();
        assert_eq!(
            deduped.len(),
            4,
            "First dedup pass should keep all Claude events"
        );

        // 3. Reconcile
        let reconciliation = ReconciliationEngine::new();
        let raw_events: Vec<_> = deduped.iter().map(|(e, _)| e.clone()).collect();
        let fingerprints: Vec<_> = deduped.iter().map(|(_, fp)| fp.clone()).collect();
        let reconciled = reconciliation.reconcile(raw_events);

        // Quota samples have different timestamps (60s apart) and the daily aggregate
        // is a different event type, so all should remain separate.
        assert_eq!(
            reconciled.len(),
            4,
            "All Claude events remain separate after reconciliation"
        );

        // 4. Store
        let stored_count = storage.store_events(&reconciled).await.unwrap();
        assert_eq!(stored_count, 4);

        // 5. Mark seen
        dedup.mark_seen(&fingerprints).await.unwrap();

        // 6. Query: verify the daily aggregate token count
        let range = TimeRange {
            start: "2024-06-30T00:00:00Z".parse().unwrap(),
            end: "2024-07-02T00:00:00Z".parse().unwrap(),
        };
        let records = storage
            .get_history(&range, Granularity::Daily)
            .await
            .unwrap();
        let claude_record = records.iter().find(|r| r.provider_id == "claude").unwrap();

        // total_tokens comes only from buddy-tokens (75000). QuotaSamples have no tokens.
        assert_eq!(claude_record.total_tokens, Some(75000));

        storage.close().await;
    });
}

/// Test the full pipeline with BOTH providers, verifying correct cross-provider aggregation.
#[test]
fn test_full_pipeline_combined_providers() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let tmp = TempDir::new().unwrap();
        let codex_dir = tmp.path().join("codex");
        let claude_dir = tmp.path().join("claude");

        // Setup fixtures for both providers
        setup_codex_fixtures(&codex_dir);
        setup_claude_fixtures(&claude_dir);

        // 1. Collect from both adapters
        let codex_config = CodexConfig {
            sessions_dir: codex_dir.join("sessions"),
            state_db_path: codex_dir.join("nonexistent.sqlite"),
            enabled: true,
        };
        let codex_adapter = CodexAdapter::new(&codex_config);
        let claude_adapter = ClaudeAdapter::new(claude_dir.clone());

        let codex_result = codex_adapter.collect(None).unwrap();
        let claude_result = claude_adapter.collect(None).unwrap();

        let mut all_events = Vec::new();
        all_events.extend(codex_result.events);
        all_events.extend(claude_result.events);

        // Total: 5 codex + 4 claude = 9 events
        assert_eq!(
            all_events.len(),
            9,
            "Combined collection should yield 9 events"
        );

        // 2. Deduplicate
        let db_path = tmp.path().join("test_combined.db");
        let storage = StorageLayer::new(&db_path).await.unwrap();
        let pool = Arc::new(storage.pool().clone());
        let mut dedup = DeduplicationEngine::new(pool).await.unwrap();

        let deduped = dedup.deduplicate(all_events).await.unwrap();
        assert_eq!(deduped.len(), 9, "All 9 unique events should pass dedup");

        // 3. Reconcile
        let reconciliation = ReconciliationEngine::new();
        let raw_events: Vec<_> = deduped.iter().map(|(e, _)| e.clone()).collect();
        let fingerprints: Vec<_> = deduped.iter().map(|(_, fp)| fp.clone()).collect();
        let reconciled = reconciliation.reconcile(raw_events);

        // Events from different providers never merge (different provider_id).
        // Within each provider, timestamps are far enough apart (>1s) so no merging.
        assert_eq!(
            reconciled.len(),
            9,
            "All events should be separate after reconciliation"
        );

        // 4. Store
        let stored_count = storage.store_events(&reconciled).await.unwrap();
        assert_eq!(stored_count, 9);

        // 5. Mark seen
        dedup.mark_seen(&fingerprints).await.unwrap();

        // 6. Query and verify cross-provider aggregation
        let range = TimeRange {
            start: "2024-06-30T00:00:00Z".parse().unwrap(),
            end: "2024-07-02T00:00:00Z".parse().unwrap(),
        };
        let records = storage
            .get_history(&range, Granularity::Daily)
            .await
            .unwrap();

        // Find provider records
        let codex_record = records.iter().find(|r| r.provider_id == "codex");
        let claude_record = records.iter().find(|r| r.provider_id == "claude");

        assert!(codex_record.is_some(), "Should have a codex record");
        assert!(claude_record.is_some(), "Should have a claude record");

        let codex_total = codex_record.unwrap().total_tokens.unwrap_or(0);
        let claude_total = claude_record.unwrap().total_tokens.unwrap_or(0);

        // Codex tokens: 450 + 600 + 1500 + 240 + 370 = 3160
        assert_eq!(codex_total, 3160, "Codex total tokens should be 3160");

        // Claude tokens: 75000 (from buddy-tokens; quota samples don't contribute tokens)
        assert_eq!(claude_total, 75000, "Claude total tokens should be 75000");

        // Total across all providers = 3160 + 75000 = 78160
        let overall_total: i64 = records.iter().filter_map(|r| r.total_tokens).sum();
        assert_eq!(
            overall_total, 78160,
            "Total tokens across all providers should be 78160"
        );

        // 7. Verify get_current_summary (uses today's date so we test the method works)
        let summary = storage.get_current_summary().await.unwrap();
        // Since fixture data is from 2024-07-01, it won't show in "today" unless today is that date.
        // But the method should not error.
        assert!(summary.last_updated <= Utc::now() + Duration::seconds(1));

        storage.close().await;
    });
}

/// Test that deduplication correctly prevents duplicate events across multiple collection cycles.
#[test]
fn test_dedup_prevents_duplicates_across_cycles() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let tmp = TempDir::new().unwrap();
        let claude_dir = tmp.path().join("claude");
        setup_claude_fixtures(&claude_dir);

        let db_path = tmp.path().join("test_dedup_cycles.db");
        let storage = StorageLayer::new(&db_path).await.unwrap();
        let pool = Arc::new(storage.pool().clone());
        let mut dedup = DeduplicationEngine::new(pool).await.unwrap();
        let reconciliation = ReconciliationEngine::new();

        // First cycle: collect and store
        let adapter = ClaudeAdapter::new(claude_dir.clone());
        let result1 = adapter.collect(None).unwrap();
        let events1 = result1.events;
        assert_eq!(events1.len(), 4);

        let deduped1 = dedup.deduplicate(events1.clone()).await.unwrap();
        assert_eq!(deduped1.len(), 4);

        let raw1: Vec<_> = deduped1.iter().map(|(e, _)| e.clone()).collect();
        let fps1: Vec<_> = deduped1.iter().map(|(_, fp)| fp.clone()).collect();
        let reconciled1 = reconciliation.reconcile(raw1);
        storage.store_events(&reconciled1).await.unwrap();
        dedup.mark_seen(&fps1).await.unwrap();

        // Second cycle: same events should all be rejected by dedup
        // Create a new adapter (simulates fresh collection from same files)
        let adapter2 = ClaudeAdapter::new(claude_dir.clone());
        let result2 = adapter2.collect(None).unwrap();
        let events2 = result2.events;
        // Claude doesn't do incremental reads based on stored checkpoint in adapter state
        // (checkpoint was in first adapter instance), so it re-reads all samples
        assert_eq!(events2.len(), 4);

        let deduped2 = dedup.deduplicate(events2).await.unwrap();
        assert_eq!(
            deduped2.len(),
            0,
            "Second cycle should reject all duplicates"
        );

        storage.close().await;
    });
}

/// Test that hourly granularity query correctly separates events into time buckets.
#[test]
fn test_hourly_aggregation_granularity() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let tmp = TempDir::new().unwrap();
        let codex_dir = tmp.path().join("codex");
        setup_codex_fixtures(&codex_dir);

        let config = CodexConfig {
            sessions_dir: codex_dir.join("sessions"),
            state_db_path: codex_dir.join("nonexistent.sqlite"),
            enabled: true,
        };
        let codex_adapter = CodexAdapter::new(&config);
        let result = codex_adapter.collect(None).unwrap();

        let db_path = tmp.path().join("test_hourly.db");
        let storage = StorageLayer::new(&db_path).await.unwrap();
        let pool = Arc::new(storage.pool().clone());
        let mut dedup = DeduplicationEngine::new(pool).await.unwrap();

        let deduped = dedup.deduplicate(result.events).await.unwrap();
        let raw: Vec<_> = deduped.iter().map(|(e, _)| e.clone()).collect();
        let fps: Vec<_> = deduped.iter().map(|(_, fp)| fp.clone()).collect();
        let reconciled = ReconciliationEngine::new().reconcile(raw);
        storage.store_events(&reconciled).await.unwrap();
        dedup.mark_seen(&fps).await.unwrap();

        // Query with hourly granularity
        let range = TimeRange {
            start: "2024-07-01T00:00:00Z".parse().unwrap(),
            end: "2024-07-02T00:00:00Z".parse().unwrap(),
        };
        let records = storage
            .get_history(&range, Granularity::Hourly)
            .await
            .unwrap();

        // Session1 events are at 10:xx, session2 events are at 11:xx
        // So we expect 2 hourly buckets
        assert_eq!(
            records.len(),
            2,
            "Should have 2 hourly buckets (10:00 and 11:00)"
        );

        // 10:00 bucket: 450 + 600 + 1500 = 2550
        let hour_10 = records
            .iter()
            .find(|r| r.timestamp.contains("10:00"))
            .unwrap();
        assert_eq!(hour_10.total_tokens, Some(2550));

        // 11:00 bucket: 240 + 370 = 610
        let hour_11 = records
            .iter()
            .find(|r| r.timestamp.contains("11:00"))
            .unwrap();
        assert_eq!(hour_11.total_tokens, Some(610));

        storage.close().await;
    });
}
