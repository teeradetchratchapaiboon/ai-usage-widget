use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::CodexConfig;
use crate::error::CollectionError;
use crate::provider::{CollectionResult, ProviderAdapter, ProviderSummary, SourceMetadata};
use crate::types::{EventType, RawUsageEvent, TokenUsage};

/// Codex Desktop provider adapter.
///
/// Reads JSONL session logs from ~/.codex/sessions/YYYY/MM/DD/*.jsonl
/// and optionally reads the state_5.sqlite database for summary-level data.
pub struct CodexAdapter {
    sessions_dir: PathBuf,
    state_db_path: PathBuf,
    file_positions: Mutex<HashMap<PathBuf, u64>>,
    last_checkpoint: Mutex<Option<DateTime<Utc>>>,
    /// Newest rate limit values seen while parsing session logs.
    rate_limits: Mutex<RateLimitSnapshot>,
}

// ─── JSONL deserialization structures ───────────────────────────────────────

#[derive(Debug, Deserialize)]
struct JsonlLine {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    line_type: Option<String>,
    payload: Option<JsonlPayload>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum JsonlPayload {
    TokenCount(TokenCountPayload),
    SessionMeta(SessionMetaPayload),
    Other(serde_json::Value),
}

#[derive(Debug, Deserialize)]
struct TokenCountPayload {
    #[serde(rename = "type")]
    payload_type: Option<String>,
    info: Option<TokenCountInfo>,
    /// Plan rate limits reported next to the token counts.
    #[serde(default)]
    rate_limits: Option<RateLimits>,
}

/// Rate limit block from a `token_count` event.
///
/// Codex interleaves several limit families (`codex`, `premium`, ...) and only
/// fills `primary`/`secondary` for the one that applies, so both windows are
/// optional and the newest non-null value wins.
#[derive(Debug, Deserialize)]
struct RateLimits {
    #[serde(default)]
    primary: Option<RateLimitWindow>,
    #[serde(default)]
    secondary: Option<RateLimitWindow>,
}

/// One rate limit window (percentage consumed plus when it resets).
#[derive(Debug, Clone, Deserialize)]
struct RateLimitWindow {
    #[serde(default)]
    used_percent: Option<f64>,
    #[serde(default)]
    window_minutes: Option<u64>,
    /// Unix timestamp (seconds) at which the window resets.
    #[serde(default)]
    resets_at: Option<i64>,
}

/// Latest rate limit snapshot kept for the widget summary.
///
/// Serialized into the provider's persisted state: the values only appear
/// while parsing new log lines, so without persistence a restart would show no
/// quota until the next Codex session writes to disk.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RateLimitSnapshot {
    pub primary_used_pct: Option<f64>,
    pub secondary_used_pct: Option<f64>,
    pub resets_at: Option<DateTime<Utc>>,
    pub window_minutes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TokenCountInfo {
    total_token_usage: Option<TokenUsageData>,
    last_token_usage: Option<TokenUsageData>,
    model_context_window: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TokenUsageData {
    input_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    reasoning_output_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct SessionMetaPayload {
    #[serde(rename = "type")]
    payload_type: Option<String>,
    model: Option<String>,
}

// ─── SQLite thread row ──────────────────────────────────────────────────────

#[derive(Debug)]
struct ThreadRow {
    id: String,
    #[allow(dead_code)]
    title: Option<String>,
    model: Option<String>,
    tokens_used: Option<i64>,
    #[allow(dead_code)]
    model_provider: Option<String>,
    cwd: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

// ─── Implementation ─────────────────────────────────────────────────────────

impl CodexAdapter {
    /// Create a new CodexAdapter from configuration.
    pub fn new(config: &CodexConfig) -> Self {
        Self {
            sessions_dir: config.sessions_dir.clone(),
            state_db_path: config.state_db_path.clone(),
            file_positions: Mutex::new(HashMap::new()),
            last_checkpoint: Mutex::new(None),
            rate_limits: Mutex::new(RateLimitSnapshot::default()),
        }
    }

    /// Discover all JSONL files in the sessions directory structure.
    /// Expected layout: sessions/YYYY/MM/DD/*.jsonl
    fn discover_jsonl_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if !self.sessions_dir.exists() {
            return files;
        }
        // Walk YYYY/MM/DD/*.jsonl
        if let Ok(years) = fs::read_dir(&self.sessions_dir) {
            for year_entry in years.flatten() {
                let year_path = year_entry.path();
                if !year_path.is_dir() {
                    continue;
                }
                if let Ok(months) = fs::read_dir(&year_path) {
                    for month_entry in months.flatten() {
                        let month_path = month_entry.path();
                        if !month_path.is_dir() {
                            continue;
                        }
                        if let Ok(days) = fs::read_dir(&month_path) {
                            for day_entry in days.flatten() {
                                let day_path = day_entry.path();
                                if !day_path.is_dir() {
                                    continue;
                                }
                                if let Ok(jsonl_files) = fs::read_dir(&day_path) {
                                    for f in jsonl_files.flatten() {
                                        let p = f.path();
                                        if p.extension().and_then(|e| e.to_str()) == Some("jsonl")
                                        {
                                            files.push(p);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        files
    }

    /// Fill the rate limit snapshot from the newest session log when it is
    /// still empty.
    ///
    /// Rate limits only appear while parsing new lines, so after a restart with
    /// restored offsets (and no persisted snapshot) the widget would show no
    /// Codex quota until the next Codex session writes to disk. This reads only
    /// the tail of the newest file and emits no events.
    fn prime_rate_limits(&self) {
        const TAIL_BYTES: u64 = 256 * 1024;

        {
            let snapshot = self.rate_limits.lock().unwrap();
            if snapshot.primary_used_pct.is_some() || snapshot.secondary_used_pct.is_some() {
                return;
            }
        }

        let newest = self
            .discover_jsonl_files()
            .into_iter()
            .filter_map(|path| {
                let modified = fs::metadata(&path).and_then(|m| m.modified()).ok()?;
                Some((modified, path))
            })
            .max_by_key(|(modified, _)| *modified)
            .map(|(_, path)| path);

        let Some(path) = newest else {
            return;
        };

        let Ok(file_size) = fs::metadata(&path).map(|m| m.len()) else {
            return;
        };

        let Ok(mut file) = fs::File::open(&path) else {
            return;
        };
        if file
            .seek(SeekFrom::Start(file_size.saturating_sub(TAIL_BYTES)))
            .is_err()
        {
            return;
        }

        let mut found = RateLimitSnapshot::default();
        for line in BufReader::new(&mut file).lines().map_while(Result::ok) {
            let Ok(parsed) = serde_json::from_str::<JsonlLine>(&line) else {
                continue;
            };
            let Some(JsonlPayload::TokenCount(tc)) = parsed.payload else {
                continue;
            };
            let Some(limits) = tc.rate_limits else {
                continue;
            };

            if let Some(primary) = limits.primary.as_ref() {
                if let Some(pct) = primary.used_percent {
                    found.primary_used_pct = Some(pct);
                    found.window_minutes = primary.window_minutes;
                    found.resets_at = primary
                        .resets_at
                        .and_then(|ts| DateTime::from_timestamp(ts, 0));
                }
            }
            if let Some(secondary) = limits.secondary.as_ref() {
                if let Some(pct) = secondary.used_percent {
                    found.secondary_used_pct = Some(pct);
                }
            }
        }

        if found.primary_used_pct.is_some() || found.secondary_used_pct.is_some() {
            log::info!(
                "Primed Codex rate limits from {}: primary={:?}%",
                path.display(),
                found.primary_used_pct
            );
            *self.rate_limits.lock().unwrap() = found;
        }
    }

    /// Read a JSONL file incrementally from the stored byte offset.
    /// Returns parsed events and the new byte offset.
    fn read_jsonl_incremental(
        &self,
        path: &Path,
    ) -> Result<(Vec<RawUsageEvent>, u64), CollectionError> {
        let metadata = fs::metadata(path)?;
        let file_size = metadata.len();

        let mut positions = self.file_positions.lock().unwrap();
        let stored_offset = positions.get(path).copied().unwrap_or(0);

        // If file size < stored offset, the file was rotated — reset to 0
        let offset = if file_size < stored_offset {
            0
        } else {
            stored_offset
        };

        let mut file = fs::File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;

        let reader = BufReader::new(&mut file);
        let mut events = Vec::new();
        let mut current_model: Option<String> = None;
        let mut bytes_read = offset;

        // Derive session hash from the file name (stem)
        let session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        let session_hash = hash_string(session_id);

        // Derive project hash from parent directory path
        let project_hash = path
            .parent()
            .and_then(|p| p.to_str())
            .map(|s| hash_string(s));

        for line_result in reader.lines() {
            let line = match line_result {
                Ok(l) => l,
                Err(_) => continue,
            };
            bytes_read += line.len() as u64 + 1; // +1 for newline

            if line.trim().is_empty() {
                continue;
            }

            let parsed: JsonlLine = match serde_json::from_str(&line) {
                Ok(p) => p,
                Err(e) => {
                    // Skip malformed lines with a warning (logged internally)
                    eprintln!(
                        "[codex] warning: skipping malformed JSONL line in {}: {}",
                        path.display(),
                        e
                    );
                    continue;
                }
            };

            let timestamp = match &parsed.timestamp {
                Some(ts) => match ts.parse::<DateTime<Utc>>() {
                    Ok(dt) => dt,
                    Err(_) => Utc::now(),
                },
                None => Utc::now(),
            };

            match (&parsed.line_type, &parsed.payload) {
                (Some(t), Some(JsonlPayload::SessionMeta(meta)))
                    if t == "session_meta" =>
                {
                    if let Some(ref model) = meta.model {
                        current_model = Some(model.clone());
                    }
                }
                (Some(t), Some(JsonlPayload::TokenCount(tc)))
                    if t == "event_msg"
                        && tc.payload_type.as_deref() == Some("token_count") =>
                {
                    // Remember the newest rate limit values. Codex reports
                    // several limit families and nulls the windows that do not
                    // apply, so only non-null windows update the snapshot.
                    if let Some(ref limits) = tc.rate_limits {
                        let mut snapshot = self.rate_limits.lock().unwrap();
                        if let Some(primary) = limits.primary.as_ref() {
                            if let Some(pct) = primary.used_percent {
                                snapshot.primary_used_pct = Some(pct);
                                snapshot.window_minutes = primary.window_minutes;
                                snapshot.resets_at = primary
                                    .resets_at
                                    .and_then(|ts| DateTime::from_timestamp(ts, 0));
                            }
                        }
                        if let Some(secondary) = limits.secondary.as_ref() {
                            if let Some(pct) = secondary.used_percent {
                                snapshot.secondary_used_pct = Some(pct);
                            }
                        }
                    }

                    if let Some(ref info) = tc.info {
                        // Prefer last_token_usage for per-event granularity
                        let usage_data = info
                            .last_token_usage
                            .as_ref()
                            .or(info.total_token_usage.as_ref());

                        if let Some(data) = usage_data {
                            let event = RawUsageEvent {
                                provider_id: "codex".to_string(),
                                event_type: EventType::TokenCount,
                                timestamp,
                                model: current_model.clone(),
                                tokens: TokenUsage {
                                    input_tokens: data.input_tokens,
                                    cached_input_tokens: data.cached_input_tokens,
                                    output_tokens: data.output_tokens,
                                    reasoning_tokens: data.reasoning_output_tokens,
                                    total_tokens: data.total_tokens,
                                },
                                context_window: info.model_context_window,
                                quota: {
                                    let snapshot = self.rate_limits.lock().unwrap();
                                    if snapshot.primary_used_pct.is_some()
                                        || snapshot.secondary_used_pct.is_some()
                                    {
                                        Some(crate::types::QuotaUsage {
                                            fast_hours_pct: snapshot.primary_used_pct,
                                            standard_pct: snapshot.secondary_used_pct,
                                            excess_pct: None,
                                            daily_tokens: None,
                                        })
                                    } else {
                                        None
                                    }
                                },
                                session_hash: Some(session_hash.clone()),
                                project_hash: project_hash.clone(),
                                source_file: Some(
                                    path.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("unknown")
                                        .to_string(),
                                ),
                                raw_metadata: None,
                            };
                            events.push(event);
                        }
                    }
                }
                _ => {
                    // Unrecognized event type — skip
                }
            }
        }

        // Update stored offset
        positions.insert(path.to_path_buf(), bytes_read);

        Ok((events, bytes_read))
    }

    /// Read thread summaries from state_5.sqlite with lock retry logic.
    /// Retries 3 times with exponential backoff (100ms, 200ms, 400ms).
    /// Falls back to empty vec if database is locked or unavailable.
    fn read_thread_summaries(
        &self,
        since: Option<DateTime<Utc>>,
    ) -> Vec<RawUsageEvent> {
        if !self.state_db_path.exists() {
            return Vec::new();
        }

        let delays_ms = [100u64, 200, 400];

        for (attempt, delay) in delays_ms.iter().enumerate() {
            match self.try_read_threads(since) {
                Ok(events) => return events,
                Err(e) => {
                    eprintln!(
                        "[codex] SQLite read attempt {}/{} failed: {}",
                        attempt + 1,
                        delays_ms.len(),
                        e
                    );
                    if attempt < delays_ms.len() - 1 {
                        std::thread::sleep(std::time::Duration::from_millis(*delay));
                    }
                }
            }
        }

        // All retries exhausted — fall back to JSONL-only
        eprintln!("[codex] warning: SQLite unavailable after 3 retries, using JSONL-only");
        Vec::new()
    }

    /// Attempt to read threads from the SQLite database.
    fn try_read_threads(
        &self,
        since: Option<DateTime<Utc>>,
    ) -> Result<Vec<RawUsageEvent>, String> {
        // Use rusqlite-style synchronous access via sqlite3
        // Since sqlx is async and we need sync here, use a minimal
        // sqlite3 connection via the sqlite3 bundled in sqlx.
        // For simplicity we use a raw connection approach.
        let db_path = self.state_db_path.to_str().ok_or("invalid db path")?;

        let conn = rusqlite_connect(db_path)?;

        let query = if since.is_some() {
            "SELECT id, title, model, tokens_used, model_provider, cwd, created_at, updated_at \
             FROM threads WHERE updated_at > ?1 ORDER BY updated_at ASC"
        } else {
            "SELECT id, title, model, tokens_used, model_provider, cwd, created_at, updated_at \
             FROM threads ORDER BY updated_at ASC"
        };

        let rows = read_thread_rows(&conn, query, since)?;

        let events: Vec<RawUsageEvent> = rows
            .into_iter()
            .filter_map(|row| {
                let timestamp = row
                    .updated_at
                    .as_deref()
                    .or(row.created_at.as_deref())
                    .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                    .unwrap_or_else(Utc::now);

                let session_hash = hash_string(&row.id);
                let project_hash = row.cwd.as_deref().map(hash_string);

                Some(RawUsageEvent {
                    provider_id: "codex".to_string(),
                    event_type: EventType::SessionSummary,
                    timestamp,
                    model: row.model,
                    tokens: TokenUsage {
                        input_tokens: None,
                        cached_input_tokens: None,
                        output_tokens: None,
                        reasoning_tokens: None,
                        total_tokens: row.tokens_used.map(|t| t as u64),
                    },
                    context_window: None,
                    quota: None,
                    session_hash: Some(session_hash),
                    project_hash,
                    source_file: Some("state_5.sqlite".to_string()),
                    raw_metadata: None,
                })
            })
            .collect();

        Ok(events)
    }
}

// ─── ProviderAdapter trait implementation ───────────────────────────────────

impl ProviderAdapter for CodexAdapter {
    fn provider_id(&self) -> &str {
        "codex"
    }

    fn display_name(&self) -> &str {
        "Codex Desktop"
    }

    fn is_available(&self) -> bool {
        self.sessions_dir.exists() || self.state_db_path.exists()
    }

    fn collect(&self, _since: Option<DateTime<Utc>>) -> Result<CollectionResult, CollectionError> {
        // Quota is only present in newly parsed lines; make sure a restart with
        // nothing new to read still knows the current limit.
        self.prime_rate_limits();

        let mut all_events = Vec::new();
        let mut files_read: u32 = 0;
        let mut bytes_processed: u64 = 0;
        let mut errors: Vec<String> = Vec::new();

        // Collect from JSONL files
        let jsonl_files = self.discover_jsonl_files();
        for file_path in &jsonl_files {
            match self.read_jsonl_incremental(file_path) {
                Ok((events, new_offset)) => {
                    files_read += 1;
                    bytes_processed += new_offset;
                    all_events.extend(events);
                }
                Err(e) => {
                    errors.push(format!("{}: {}", file_path.display(), e));
                }
            }
        }

        // Collect from SQLite (with retry)
        let db_events = self.read_thread_summaries(_since);
        if !db_events.is_empty() {
            files_read += 1;
        }
        all_events.extend(db_events);

        // Update checkpoint
        let checkpoint = all_events
            .iter()
            .map(|e| e.timestamp)
            .max()
            .unwrap_or_else(Utc::now);

        {
            let mut cp = self.last_checkpoint.lock().unwrap();
            *cp = Some(checkpoint);
        }

        Ok(CollectionResult {
            events: all_events,
            checkpoint,
            source_metadata: SourceMetadata {
                files_read,
                bytes_processed,
                errors,
            },
        })
    }

    fn get_current_summary(&self) -> Result<ProviderSummary, CollectionError> {
        let snapshot = self.rate_limits.lock().unwrap().clone();
        let quota = if snapshot.primary_used_pct.is_some() || snapshot.secondary_used_pct.is_some()
        {
            Some(crate::types::QuotaUsage {
                fast_hours_pct: snapshot.primary_used_pct,
                standard_pct: snapshot.secondary_used_pct,
                excess_pct: None,
                daily_tokens: None,
            })
        } else {
            None
        };

        Ok(ProviderSummary {
            provider_id: "codex".to_string(),
            display_name: "Codex Desktop".to_string(),
            is_available: self.is_available(),
            current_model: None,
            tokens_today: None,
            quota,
            context_window: None,
            last_activity: *self.last_checkpoint.lock().unwrap(),
            quota_resets_at: snapshot.resets_at,
        })
    }

    fn quota_resets_at(&self) -> Option<DateTime<Utc>> {
        self.rate_limits.lock().unwrap().resets_at
    }

    fn last_checkpoint(&self) -> Option<DateTime<Utc>> {
        *self.last_checkpoint.lock().unwrap()
    }

    fn export_state(&self) -> crate::provider::ProviderState {
        let positions = self.file_positions.lock().unwrap();
        let snapshot = self.rate_limits.lock().unwrap().clone();

        crate::provider::ProviderState {
            file_positions: positions
                .iter()
                .map(|(path, offset)| (path.to_string_lossy().to_string(), *offset))
                .collect(),
            checkpoint: *self.last_checkpoint.lock().unwrap(),
            metadata: if snapshot.primary_used_pct.is_some()
                || snapshot.secondary_used_pct.is_some()
            {
                serde_json::to_string(&snapshot).ok()
            } else {
                None
            },
        }
    }

    fn restore_state(&self, state: &crate::provider::ProviderState) {
        let mut positions = self.file_positions.lock().unwrap();
        for (path, offset) in &state.file_positions {
            positions.insert(PathBuf::from(path), *offset);
        }
        if state.checkpoint.is_some() {
            *self.last_checkpoint.lock().unwrap() = state.checkpoint;
        }
        if let Some(raw) = state.metadata.as_deref() {
            match serde_json::from_str::<RateLimitSnapshot>(raw) {
                Ok(snapshot) => *self.rate_limits.lock().unwrap() = snapshot,
                Err(e) => log::warn!("Ignoring unreadable codex state metadata: {}", e),
            }
        }
    }
}

// ─── Helper functions ───────────────────────────────────────────────────────

/// Hash a string with SHA-256 and return a hex-encoded hash.
/// Used to anonymize project paths and session IDs.
fn hash_string(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Open a read-only SQLite connection to the Codex state database.
fn rusqlite_connect(path: &str) -> Result<rusqlite::Connection, String> {
    if !Path::new(path).exists() {
        return Err(format!("database file not found: {}", path));
    }
    rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("failed to open database: {}", e))
}

/// Read thread rows from the SQLite database.
fn read_thread_rows(
    conn: &rusqlite::Connection,
    query: &str,
    since: Option<DateTime<Utc>>,
) -> Result<Vec<ThreadRow>, String> {
    let mut stmt = conn
        .prepare(query)
        .map_err(|e| format!("prepare failed: {}", e))?;

    let row_mapper = |row: &rusqlite::Row| -> rusqlite::Result<ThreadRow> {
        Ok(ThreadRow {
            id: row.get(0)?,
            title: row.get(1)?,
            model: row.get(2)?,
            tokens_used: row.get(3)?,
            model_provider: row.get(4)?,
            cwd: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    };

    let rows = if since.is_some() {
        let since_str = since.unwrap().to_rfc3339();
        stmt.query_map(rusqlite::params![since_str], row_mapper)
    } else {
        stmt.query_map([], row_mapper)
    }
    .map_err(|e| format!("query failed: {}", e))?;

    let mut result = Vec::new();
    for row in rows {
        match row {
            Ok(r) => result.push(r),
            Err(e) => {
                eprintln!("[codex] warning: skipping thread row: {}", e);
            }
        }
    }

    Ok(result)
}


#[cfg(test)]
mod rate_limit_tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parses_rate_limits_from_token_count() {
        // Codex reports plan limits next to the token counts; a full weekly
        // window shows up as primary.used_percent = 100.
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("2026").join("07").join("30");
        fs::create_dir_all(&day).unwrap();

        let line = r#"{"timestamp":"2026-07-30T20:54:57.240Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15},"model_context_window":258400},"rate_limits":{"limit_id":"codex","primary":{"used_percent":100.0,"window_minutes":10080,"resets_at":1785922677},"secondary":{"used_percent":42.5}}}}"#;
        fs::write(day.join("rollout-test.jsonl"), format!("{}
", line)).unwrap();

        let config = CodexConfig {
            sessions_dir: dir.path().to_path_buf(),
            state_db_path: dir.path().join("missing.sqlite"),
            enabled: true,
        };
        let adapter = CodexAdapter::new(&config);
        let result = adapter.collect(None).unwrap();

        let quota = result.events[0]
            .quota
            .as_ref()
            .expect("event should carry the quota");
        assert_eq!(quota.fast_hours_pct, Some(100.0));
        assert_eq!(quota.standard_pct, Some(42.5));

        let summary = adapter.get_current_summary().unwrap();
        let summary_quota = summary.quota.expect("summary should carry the quota");
        assert_eq!(summary_quota.fast_hours_pct, Some(100.0));
        assert_eq!(
            adapter.quota_resets_at().map(|dt| dt.timestamp()),
            Some(1785922677)
        );
    }

    #[test]
    fn test_null_rate_limit_windows_keep_previous_values() {
        // Later lines carry a different limit family with null windows; they
        // must not wipe the values already seen.
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("2026").join("07").join("30");
        fs::create_dir_all(&day).unwrap();

        let with_limits = r#"{"timestamp":"2026-07-30T20:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":10}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":100.0,"window_minutes":10080,"resets_at":1785922677},"secondary":null}}}"#;
        let without = r#"{"timestamp":"2026-07-30T20:54:57.240Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":10}},"rate_limits":{"limit_id":"premium","primary":null,"secondary":null}}}"#;
        fs::write(
            day.join("rollout-test.jsonl"),
            format!("{}
{}
", with_limits, without),
        )
        .unwrap();

        let config = CodexConfig {
            sessions_dir: dir.path().to_path_buf(),
            state_db_path: dir.path().join("missing.sqlite"),
            enabled: true,
        };
        let adapter = CodexAdapter::new(&config);
        adapter.collect(None).unwrap();

        let summary = adapter.get_current_summary().unwrap();
        assert_eq!(
            summary.quota.and_then(|q| q.fast_hours_pct),
            Some(100.0),
            "a null window must not erase the last known percentage"
        );
    }

    #[test]
    fn test_rate_limit_snapshot_survives_restart() {
        // Offsets are restored on startup, so no new lines are parsed and the
        // in-memory snapshot would be empty without persisting it.
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("2026").join("07").join("30");
        fs::create_dir_all(&day).unwrap();
        let line = r#"{"timestamp":"2026-07-30T20:00:00.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":10}},"rate_limits":{"limit_id":"codex","primary":{"used_percent":100.0,"window_minutes":10080,"resets_at":1785922677}}}}"#;
        fs::write(day.join("rollout-test.jsonl"), format!("{}
", line)).unwrap();

        let config = CodexConfig {
            sessions_dir: dir.path().to_path_buf(),
            state_db_path: dir.path().join("missing.sqlite"),
            enabled: true,
        };

        let first = CodexAdapter::new(&config);
        first.collect(None).unwrap();
        let state = first.export_state();
        assert!(state.metadata.is_some(), "quota should be exported");

        // A fresh adapter that only restores state (no re-read) still reports it
        let restarted = CodexAdapter::new(&config);
        restarted.restore_state(&state);

        let summary = restarted.get_current_summary().unwrap();
        assert_eq!(summary.quota.and_then(|q| q.fast_hours_pct), Some(100.0));
        assert_eq!(
            restarted.quota_resets_at().map(|dt| dt.timestamp()),
            Some(1785922677)
        );
    }
}

#[cfg(test)]
mod prop_tests_jsonl_reader {
    //! Property-based tests for JSONL incremental reader correctness.
    //!
    //! **Validates: Requirements 1.3, 1.4, 1.5, 13.6**

    use super::*;
    use proptest::prelude::*;
    use std::io::Write;
    use tempfile::TempDir;

    /// Generate a valid token_count JSONL line with arbitrary token values.
    fn valid_token_line(input: u64, output: u64, total: u64) -> String {
        format!(
            r#"{{"timestamp":"2024-01-15T10:00:01Z","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"input_tokens":{},"output_tokens":{},"total_tokens":{}}},"model_context_window":128000}}}}}}"#,
            input, output, total
        )
    }

    /// Generate a valid session_meta JSONL line.
    fn session_meta_line(model: &str) -> String {
        format!(
            r#"{{"timestamp":"2024-01-15T10:00:00Z","type":"session_meta","payload":{{"type":"session_meta","model":"{}"}}}}"#,
            model
        )
    }

    /// Create a CodexAdapter pointing to a temporary directory with proper structure.
    fn create_test_adapter(sessions_dir: &Path) -> CodexAdapter {
        let config = CodexConfig {
            sessions_dir: sessions_dir.to_path_buf(),
            state_db_path: sessions_dir.join("nonexistent.sqlite"),
            enabled: true,
        };
        CodexAdapter::new(&config)
    }

    /// Write content to a JSONL file inside the temp dir and return the path.
    fn write_jsonl_file(dir: &Path, filename: &str, content: &str) -> PathBuf {
        let file_path = dir.join(filename);
        let mut file = fs::File::create(&file_path).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file_path
    }

    // ─── Property 1: Incremental read yields all events ─────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(20))]

        /// Reading a JSONL file from offset 0 yields all parseable events.
        /// Reading incrementally (first half, then second half) yields the same
        /// total events as reading the whole file from 0.
        #[test]
        fn incremental_read_yields_all_events(
            event_count in 1u32..8,
            input_tokens in proptest::collection::vec(1u64..10000, 1..8),
            output_tokens in proptest::collection::vec(1u64..10000, 1..8),
        ) {
            let event_count = event_count as usize;
            let tmp_dir = TempDir::new().unwrap();
            let sessions_dir = tmp_dir.path();

            // Build JSONL content with session_meta + N token_count events
            let mut lines = vec![session_meta_line("gpt-4")];
            for i in 0..event_count {
                let inp = input_tokens[i % input_tokens.len()];
                let outp = output_tokens[i % output_tokens.len()];
                lines.push(valid_token_line(inp, outp, inp + outp));
            }
            let content = lines.join("\n") + "\n";

            let file_path = write_jsonl_file(sessions_dir, "test.jsonl", &content);

            // Full read from offset 0
            let adapter = create_test_adapter(sessions_dir);
            let (full_events, full_offset) = adapter.read_jsonl_incremental(&file_path).unwrap();
            prop_assert_eq!(full_events.len(), event_count);
            prop_assert_eq!(full_offset, content.len() as u64);

            // Incremental read: split the content at roughly the midpoint (on a newline boundary)
            let mid = content.len() / 2;
            // Find next newline after mid
            let split_pos = content[mid..].find('\n').map(|p| mid + p + 1).unwrap_or(content.len());

            let first_half = &content[..split_pos];
            let file_path2 = write_jsonl_file(sessions_dir, "test2.jsonl", first_half);

            let adapter2 = create_test_adapter(sessions_dir);
            let (first_events, first_offset) = adapter2.read_jsonl_incremental(&file_path2).unwrap();

            // Now write the full content and read from the stored offset
            fs::write(&file_path2, &content).unwrap();
            let (second_events, _) = adapter2.read_jsonl_incremental(&file_path2).unwrap();

            let total_incremental = first_events.len() + second_events.len();
            prop_assert_eq!(total_incremental, event_count);
            prop_assert_eq!(first_offset, split_pos as u64);
        }
    }

    // ─── Property 2: Offset reset on file shrink ────────────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(20))]

        /// If file size < stored offset, the offset resets to 0 and the entire
        /// file is re-read.
        #[test]
        fn offset_resets_on_file_shrink(
            event_count in 2u32..6,
            input_tokens in 1u64..10000,
            output_tokens in 1u64..10000,
        ) {
            let event_count = event_count as usize;
            let tmp_dir = TempDir::new().unwrap();
            let sessions_dir = tmp_dir.path();

            // Build a larger file first
            let mut lines = vec![session_meta_line("o1-mini")];
            for _ in 0..event_count {
                lines.push(valid_token_line(input_tokens, output_tokens, input_tokens + output_tokens));
            }
            let large_content = lines.join("\n") + "\n";
            let file_path = write_jsonl_file(sessions_dir, "shrink.jsonl", &large_content);

            let adapter = create_test_adapter(sessions_dir);

            // First read - processes the whole file and stores offset
            let (events1, offset1) = adapter.read_jsonl_incremental(&file_path).unwrap();
            prop_assert_eq!(events1.len(), event_count);
            prop_assert!(offset1 > 0);

            // Now shrink the file (write fewer events)
            let small_lines = vec![
                session_meta_line("o1-mini"),
                valid_token_line(42, 84, 126),
            ];
            let small_content = small_lines.join("\n") + "\n";
            fs::write(&file_path, &small_content).unwrap();

            // The small file size < stored offset, so it should reset and re-read from 0
            let (events2, offset2) = adapter.read_jsonl_incremental(&file_path).unwrap();
            prop_assert_eq!(events2.len(), 1); // Only one token_count event in small file
            prop_assert_eq!(offset2, small_content.len() as u64);
        }
    }

    // ─── Property 3: Malformed lines are skipped ────────────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(20))]

        /// If a JSONL file contains invalid JSON lines, those lines are skipped
        /// and valid lines still produce events.
        #[test]
        fn malformed_lines_are_skipped(
            valid_count in 1u32..5,
            malformed_count in 1u32..5,
            input_tokens in 1u64..10000,
        ) {
            let valid_count = valid_count as usize;
            let malformed_count = malformed_count as usize;
            let tmp_dir = TempDir::new().unwrap();
            let sessions_dir = tmp_dir.path();

            let mut lines = vec![session_meta_line("gpt-4")];

            // Interleave valid and malformed lines
            for i in 0..(valid_count + malformed_count) {
                if i < valid_count {
                    lines.push(valid_token_line(input_tokens, input_tokens * 2, input_tokens * 3));
                } else {
                    // Various malformed lines
                    lines.push(format!("{{not valid json at all index {}", i));
                }
            }

            let content = lines.join("\n") + "\n";
            let file_path = write_jsonl_file(sessions_dir, "malformed.jsonl", &content);

            let adapter = create_test_adapter(sessions_dir);
            let (events, _) = adapter.read_jsonl_incremental(&file_path).unwrap();

            // Only valid token_count events should be parsed
            prop_assert_eq!(events.len(), valid_count);
        }
    }

    // ─── Property 4: Empty/blank lines are skipped ──────────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(20))]

        /// Empty lines don't produce events and don't break the parser.
        #[test]
        fn empty_lines_are_skipped(
            valid_count in 1u32..5,
            empty_count in 1u32..8,
            input_tokens in 1u64..10000,
        ) {
            let valid_count = valid_count as usize;
            let empty_count = empty_count as usize;
            let tmp_dir = TempDir::new().unwrap();
            let sessions_dir = tmp_dir.path();

            let mut lines: Vec<String> = vec![session_meta_line("claude-3")];

            // Add valid lines
            for _ in 0..valid_count {
                lines.push(valid_token_line(input_tokens, input_tokens, input_tokens * 2));
            }

            // Insert empty/blank lines throughout
            for _ in 0..empty_count {
                lines.push(String::new()); // empty line
                lines.push("   ".to_string()); // blank (whitespace-only) line
            }

            let content = lines.join("\n") + "\n";
            let file_path = write_jsonl_file(sessions_dir, "empty_lines.jsonl", &content);

            let adapter = create_test_adapter(sessions_dir);
            let (events, _) = adapter.read_jsonl_incremental(&file_path).unwrap();

            // Only valid token_count lines produce events
            prop_assert_eq!(events.len(), valid_count);
        }
    }

    // ─── Property 5: Byte offset tracking ───────────────────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(20))]

        /// After reading, the new offset equals the original offset plus the
        /// bytes of lines consumed (including newlines).
        #[test]
        fn byte_offset_tracking(
            event_count in 1u32..6,
            input_tokens in 1u64..10000,
            output_tokens in 1u64..10000,
        ) {
            let event_count = event_count as usize;
            let tmp_dir = TempDir::new().unwrap();
            let sessions_dir = tmp_dir.path();

            let mut lines = vec![session_meta_line("gpt-4o")];
            for _ in 0..event_count {
                lines.push(valid_token_line(input_tokens, output_tokens, input_tokens + output_tokens));
            }
            let content = lines.join("\n") + "\n";
            let expected_bytes = content.len() as u64;

            let file_path = write_jsonl_file(sessions_dir, "offset.jsonl", &content);

            let adapter = create_test_adapter(sessions_dir);
            let (_, offset) = adapter.read_jsonl_incremental(&file_path).unwrap();

            // Offset should equal total bytes in the file (read from 0)
            prop_assert_eq!(offset, expected_bytes);

            // Second read from stored offset should produce 0 events and same offset
            let (events2, offset2) = adapter.read_jsonl_incremental(&file_path).unwrap();
            prop_assert_eq!(events2.len(), 0);
            prop_assert_eq!(offset2, expected_bytes);
        }
    }
}
