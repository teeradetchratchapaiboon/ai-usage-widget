use std::path::Path;

use chrono::{Datelike, Duration, Local, NaiveDateTime, TimeZone, Utc};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StorageError;
use crate::query_types::{
    Granularity, ProviderUsageSummary, TimeRange, UsageRecord, UsageSummary,
};
use crate::types::ReconciledEvent;

/// Core storage layer managing SQLite database connections, schema migrations,
/// and query/aggregation operations.
pub struct StorageLayer {
    pool: SqlitePool,
    /// Data retention period in days (default: 365).
    pub retention_days: u32,
    /// Path to the SQLite database file.
    db_path: std::path::PathBuf,
}

/// Convert a naive local wall-clock time to a UTC RFC 3339 string.
///
/// Ambiguous local times (DST fall-back) resolve to the earliest match; times
/// that do not exist locally (DST spring-forward) fall back to treating the
/// value as UTC rather than failing the query.
fn local_naive_to_utc_string(naive_local: NaiveDateTime) -> String {
    Local
        .from_local_datetime(&naive_local)
        .earliest()
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(&naive_local))
        .to_rfc3339()
}

/// Start of the current day and of the current week (Monday), in *local* time,
/// returned as UTC RFC 3339 strings for use as query boundaries.
///
/// "Today" has to follow the user's clock — using UTC midnight would drop the
/// morning hours for every timezone ahead of UTC (e.g. UTC+7).
fn current_period_starts() -> (String, String) {
    let now_local = Local::now();
    let today = now_local.date_naive();

    let today_start = today.and_hms_opt(0, 0, 0).unwrap();
    let week_start = (today - Duration::days(now_local.weekday().num_days_from_monday() as i64))
        .and_hms_opt(0, 0, 0)
        .unwrap();

    (
        local_naive_to_utc_string(today_start),
        local_naive_to_utc_string(week_start),
    )
}

impl StorageLayer {
    /// Create a new StorageLayer with WAL mode enabled and a connection pool.
    ///
    /// Creates the database file and parent directories if they don't exist.
    pub async fn new(db_path: &Path) -> Result<Self, StorageError> {
        Self::with_retention(db_path, 365).await
    }

    /// Create a new StorageLayer with a custom retention period.
    pub async fn with_retention(db_path: &Path, retention_days: u32) -> Result<Self, StorageError> {
        // Ensure parent directories exist
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                StorageError::ConnectionFailed(format!(
                    "failed to create database directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        let connect_options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(connect_options)
            .await
            .map_err(|e| {
                StorageError::ConnectionFailed(format!("failed to connect to database: {}", e))
            })?;

        let storage = Self {
            pool,
            retention_days,
            db_path: db_path.to_path_buf(),
        };
        storage.run_migrations().await?;

        Ok(storage)
    }

    /// Run all database migrations (CREATE TABLE IF NOT EXISTS).
    pub async fn run_migrations(&self) -> Result<(), StorageError> {
        sqlx::raw_sql(MIGRATION_SQL)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::MigrationFailed(format!("migration failed: {}", e)))?;

        Ok(())
    }

    /// Get a reference to the underlying connection pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Close the connection pool gracefully.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Batch insert ReconciledEvents into the database using a transaction for atomicity.
    ///
    /// Returns the number of events inserted. Uses INSERT OR IGNORE to skip
    /// events that already exist (by primary key).
    pub async fn store_events(&self, events: &[ReconciledEvent]) -> Result<u32, StorageError> {
        if events.is_empty() {
            return Ok(0);
        }

        let mut tx = self.pool.begin().await.map_err(|e| {
            StorageError::QueryFailed(format!("failed to begin transaction: {}", e))
        })?;

        let mut count: u32 = 0;

        for event in events {
            let id = event.id.to_string();
            let fingerprint = event.fingerprint.0.as_slice();
            let provider_id = &event.provider_id;
            let event_type = serde_json::to_string(&event.event_type).unwrap_or_default();
            // Strip quotes from serialized enum string (e.g., "\"token_count\"" -> "token_count")
            let event_type = event_type.trim_matches('"');
            let timestamp_utc = event.timestamp.to_rfc3339();
            let model = event.model.as_deref();
            let input_tokens = event.tokens.input_tokens.map(|v| v as i64);
            let cached_input_tokens = event.tokens.cached_input_tokens.map(|v| v as i64);
            let output_tokens = event.tokens.output_tokens.map(|v| v as i64);
            let reasoning_tokens = event.tokens.reasoning_tokens.map(|v| v as i64);
            let total_tokens = event.tokens.total_tokens.map(|v| v as i64);
            let context_window = event.context_window.map(|v| v as i64);
            let quota_fast_pct = event.quota.as_ref().and_then(|q| q.fast_hours_pct);
            let quota_standard_pct = event.quota.as_ref().and_then(|q| q.standard_pct);
            let quota_excess_pct = event.quota.as_ref().and_then(|q| q.excess_pct);
            let quota_daily_tokens = event
                .quota
                .as_ref()
                .and_then(|q| q.daily_tokens)
                .map(|v| v as i64);
            let session_hash = event.session_hash.as_deref();
            let project_hash = event.project_hash.as_deref();
            let source_count = event.source_count as i32;
            let reconciled_at = event.reconciled_at.to_rfc3339();

            sqlx::query(
                r#"INSERT OR IGNORE INTO usage_events (
                    id, fingerprint, provider_id, event_type, timestamp_utc, model,
                    input_tokens, cached_input_tokens, output_tokens, reasoning_tokens,
                    total_tokens, context_window, quota_fast_pct, quota_standard_pct,
                    quota_excess_pct, quota_daily_tokens, session_hash, project_hash,
                    source_count, reconciled_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14,
                    ?15, ?16, ?17, ?18,
                    ?19, ?20
                )"#,
            )
            .bind(&id)
            .bind(fingerprint)
            .bind(provider_id)
            .bind(event_type)
            .bind(&timestamp_utc)
            .bind(model)
            .bind(input_tokens)
            .bind(cached_input_tokens)
            .bind(output_tokens)
            .bind(reasoning_tokens)
            .bind(total_tokens)
            .bind(context_window)
            .bind(quota_fast_pct)
            .bind(quota_standard_pct)
            .bind(quota_excess_pct)
            .bind(quota_daily_tokens)
            .bind(session_hash)
            .bind(project_hash)
            .bind(source_count)
            .bind(&reconciled_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                StorageError::QueryFailed(format!("failed to insert event {}: {}", id, e))
            })?;

            count += 1;
        }

        tx.commit().await.map_err(|e| {
            StorageError::QueryFailed(format!("failed to commit transaction: {}", e))
        })?;

        Ok(count)
    }

    /// Aggregate today's and this week's tokens per provider.
    ///
    /// "Today" is defined as the current UTC date (from midnight to now).
    /// "This week" starts on Monday UTC midnight.
    pub async fn get_current_summary(&self) -> Result<UsageSummary, StorageError> {
        let (today_start_str, week_start_str) = current_period_starts();

        // Query per-provider summaries for today
        let today_rows = sqlx::query(
            r#"SELECT
                provider_id,
                SUM(input_tokens) as sum_input,
                SUM(output_tokens) as sum_output,
                SUM(total_tokens) as sum_total,
                MAX(timestamp_utc) as last_activity
            FROM usage_events
            WHERE datetime(timestamp_utc) >= datetime(?1)
            GROUP BY provider_id"#,
        )
        .bind(&today_start_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::QueryFailed(format!("today summary query failed: {}", e)))?;

        // Query per-provider summaries for this week
        let week_rows = sqlx::query(
            r#"SELECT
                provider_id,
                SUM(input_tokens) as sum_input,
                SUM(output_tokens) as sum_output,
                SUM(total_tokens) as sum_total
            FROM usage_events
            WHERE datetime(timestamp_utc) >= datetime(?1)
            GROUP BY provider_id"#,
        )
        .bind(&week_start_str)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::QueryFailed(format!("week summary query failed: {}", e)))?;

        // Build provider summaries combining today and week data
        let mut providers: Vec<ProviderUsageSummary> = Vec::new();

        // Collect all unique provider IDs from both queries
        let mut provider_ids: Vec<String> = Vec::new();
        for row in &today_rows {
            let pid: String = row.get("provider_id");
            if !provider_ids.contains(&pid) {
                provider_ids.push(pid);
            }
        }
        for row in &week_rows {
            let pid: String = row.get("provider_id");
            if !provider_ids.contains(&pid) {
                provider_ids.push(pid);
            }
        }

        for pid in &provider_ids {
            let today_data = today_rows.iter().find(|r| {
                let p: String = r.get("provider_id");
                &p == pid
            });
            let week_data = week_rows.iter().find(|r| {
                let p: String = r.get("provider_id");
                &p == pid
            });

            providers.push(ProviderUsageSummary {
                provider_id: pid.clone(),
                input_tokens_today: today_data.and_then(|r| r.get::<Option<i64>, _>("sum_input")),
                output_tokens_today: today_data
                    .and_then(|r| r.get::<Option<i64>, _>("sum_output")),
                total_tokens_today: today_data.and_then(|r| r.get::<Option<i64>, _>("sum_total")),
                input_tokens_this_week: week_data
                    .and_then(|r| r.get::<Option<i64>, _>("sum_input")),
                output_tokens_this_week: week_data
                    .and_then(|r| r.get::<Option<i64>, _>("sum_output")),
                total_tokens_this_week: week_data
                    .and_then(|r| r.get::<Option<i64>, _>("sum_total")),
                last_activity: today_data
                    .and_then(|r| r.get::<Option<String>, _>("last_activity")),
            });
        }

        // Compute totals across all providers
        let total_tokens_today: Option<i64> = {
            let sum: i64 = providers
                .iter()
                .filter_map(|p| p.total_tokens_today)
                .sum();
            if providers.iter().any(|p| p.total_tokens_today.is_some()) {
                Some(sum)
            } else {
                None
            }
        };

        let total_tokens_this_week: Option<i64> = {
            let sum: i64 = providers
                .iter()
                .filter_map(|p| p.total_tokens_this_week)
                .sum();
            if providers
                .iter()
                .any(|p| p.total_tokens_this_week.is_some())
            {
                Some(sum)
            } else {
                None
            }
        };

        Ok(UsageSummary {
            providers,
            total_tokens_today,
            total_tokens_this_week,
            last_updated: Utc::now(),
        })
    }

    /// Query usage history with time range and granularity parameters.
    ///
    /// Groups events into time buckets according to the specified granularity
    /// and returns aggregated token counts per bucket per provider.
    pub async fn get_history(
        &self,
        range: &TimeRange,
        granularity: Granularity,
    ) -> Result<Vec<UsageRecord>, StorageError> {
        let start_str = range.start.to_rfc3339();
        let end_str = range.end.to_rfc3339();
        let strftime_fmt = granularity.strftime_format();

        let query_str = format!(
            r#"SELECT
                strftime('{fmt}', timestamp_utc) as time_bucket,
                provider_id,
                SUM(input_tokens) as sum_input,
                SUM(output_tokens) as sum_output,
                SUM(total_tokens) as sum_total,
                AVG(quota_fast_pct) as avg_quota_fast,
                AVG(quota_standard_pct) as avg_quota_standard
            FROM usage_events
            WHERE datetime(timestamp_utc) >= datetime(?1)
              AND datetime(timestamp_utc) < datetime(?2)
            GROUP BY time_bucket, provider_id
            ORDER BY time_bucket ASC, provider_id ASC"#,
            fmt = strftime_fmt
        );

        let rows = sqlx::query(&query_str)
            .bind(&start_str)
            .bind(&end_str)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::QueryFailed(format!("history query failed: {}", e)))?;

        let records: Vec<UsageRecord> = rows
            .iter()
            .map(|row| UsageRecord {
                timestamp: row.get::<String, _>("time_bucket"),
                provider_id: row.get::<String, _>("provider_id"),
                model: None, // Aggregated across models at this level
                input_tokens: row.get::<Option<i64>, _>("sum_input"),
                output_tokens: row.get::<Option<i64>, _>("sum_output"),
                total_tokens: row.get::<Option<i64>, _>("sum_total"),
                quota_fast_pct: row.get::<Option<f64>, _>("avg_quota_fast"),
                quota_standard_pct: row.get::<Option<f64>, _>("avg_quota_standard"),
            })
            .collect();

        Ok(records)
    }

    /// Delete records older than the configured retention period.
    ///
    /// Returns the number of pruned event records.
    pub async fn prune_expired(&self) -> Result<u32, StorageError> {
        let cutoff = Utc::now() - Duration::days(self.retention_days as i64);
        let cutoff_str = cutoff.to_rfc3339();

        let result = sqlx::query("DELETE FROM usage_events WHERE timestamp_utc < ?1")
            .bind(&cutoff_str)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryFailed(format!("prune query failed: {}", e)))?;

        // Also prune old fingerprints to keep the seen_fingerprints table manageable
        sqlx::query("DELETE FROM seen_fingerprints WHERE first_seen_at < ?1")
            .bind(&cutoff_str)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                StorageError::QueryFailed(format!("fingerprint prune failed: {}", e))
            })?;

        Ok(result.rows_affected() as u32)
    }

    /// Persist a provider's incremental-collection state (file offsets and
    /// checkpoint) so a restart resumes instead of re-reading every file.
    pub async fn save_provider_state(
        &self,
        provider_id: &str,
        state: &crate::provider::ProviderState,
    ) -> Result<(), StorageError> {
        let now = Utc::now().to_rfc3339();

        let mut tx = self.pool.begin().await.map_err(|e| {
            StorageError::QueryFailed(format!("failed to begin state transaction: {}", e))
        })?;

        for (file_path, offset) in &state.file_positions {
            sqlx::query(
                r#"INSERT INTO file_positions (provider_id, file_path, byte_offset, last_read_at)
                   VALUES (?1, ?2, ?3, ?4)
                   ON CONFLICT(provider_id, file_path)
                   DO UPDATE SET byte_offset = excluded.byte_offset,
                                 last_read_at = excluded.last_read_at"#,
            )
            .bind(provider_id)
            .bind(file_path)
            .bind(*offset as i64)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                StorageError::QueryFailed(format!("file position upsert failed: {}", e))
            })?;
        }

        if let Some(checkpoint) = state.checkpoint {
            sqlx::query(
                r#"INSERT INTO collection_checkpoints
                       (provider_id, last_checkpoint, last_success_at, files_processed, metadata)
                   VALUES (?1, ?2, ?3, ?4, ?5)
                   ON CONFLICT(provider_id)
                   DO UPDATE SET last_checkpoint = excluded.last_checkpoint,
                                 last_success_at = excluded.last_success_at,
                                 files_processed = excluded.files_processed,
                                 metadata = COALESCE(excluded.metadata, collection_checkpoints.metadata)"#,
            )
            .bind(provider_id)
            .bind(checkpoint.to_rfc3339())
            .bind(&now)
            .bind(state.file_positions.len() as i64)
            .bind(state.metadata.as_deref())
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::QueryFailed(format!("checkpoint upsert failed: {}", e)))?;
        }

        tx.commit().await.map_err(|e| {
            StorageError::QueryFailed(format!("failed to commit state transaction: {}", e))
        })?;

        Ok(())
    }

    /// Load a provider's persisted incremental-collection state.
    pub async fn load_provider_state(
        &self,
        provider_id: &str,
    ) -> Result<crate::provider::ProviderState, StorageError> {
        let rows =
            sqlx::query("SELECT file_path, byte_offset FROM file_positions WHERE provider_id = ?1")
                .bind(provider_id)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| {
                    StorageError::QueryFailed(format!("file position query failed: {}", e))
                })?;

        let mut file_positions = std::collections::HashMap::new();
        for row in rows {
            let path: String = row.get("file_path");
            let offset: i64 = row.get("byte_offset");
            file_positions.insert(path, offset.max(0) as u64);
        }

        let checkpoint_row = sqlx::query(
            "SELECT last_checkpoint, metadata FROM collection_checkpoints WHERE provider_id = ?1",
        )
        .bind(provider_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryFailed(format!("checkpoint query failed: {}", e)))?;

        let (checkpoint, metadata) = match checkpoint_row {
            Some(row) => {
                let raw: String = row.get("last_checkpoint");
                let checkpoint = chrono::DateTime::parse_from_rfc3339(&raw)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc));
                let metadata: Option<String> = row.get("metadata");
                (checkpoint, metadata)
            }
            None => (None, None),
        };

        Ok(crate::provider::ProviderState {
            file_positions,
            checkpoint,
            metadata,
        })
    }

    /// Get per-provider statistics: today's and this week's tokens.
    pub async fn get_provider_summary(
        &self,
        provider_id: &str,
    ) -> Result<ProviderUsageSummary, StorageError> {
        let (today_start_str, week_start_str) = current_period_starts();

        // Today's stats for this provider
        let today_row = sqlx::query(
            r#"SELECT
                SUM(input_tokens) as sum_input,
                SUM(output_tokens) as sum_output,
                SUM(total_tokens) as sum_total,
                MAX(timestamp_utc) as last_activity
            FROM usage_events
            WHERE provider_id = ?1 AND datetime(timestamp_utc) >= datetime(?2)"#,
        )
        .bind(provider_id)
        .bind(&today_start_str)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            StorageError::QueryFailed(format!("provider today summary query failed: {}", e))
        })?;

        // This week's stats for this provider
        let week_row = sqlx::query(
            r#"SELECT
                SUM(input_tokens) as sum_input,
                SUM(output_tokens) as sum_output,
                SUM(total_tokens) as sum_total
            FROM usage_events
            WHERE provider_id = ?1 AND datetime(timestamp_utc) >= datetime(?2)"#,
        )
        .bind(provider_id)
        .bind(&week_start_str)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            StorageError::QueryFailed(format!("provider week summary query failed: {}", e))
        })?;

        Ok(ProviderUsageSummary {
            provider_id: provider_id.to_string(),
            input_tokens_today: today_row.get::<Option<i64>, _>("sum_input"),
            output_tokens_today: today_row.get::<Option<i64>, _>("sum_output"),
            total_tokens_today: today_row.get::<Option<i64>, _>("sum_total"),
            input_tokens_this_week: week_row.get::<Option<i64>, _>("sum_input"),
            output_tokens_this_week: week_row.get::<Option<i64>, _>("sum_output"),
            total_tokens_this_week: week_row.get::<Option<i64>, _>("sum_total"),
            last_activity: today_row.get::<Option<String>, _>("last_activity"),
        })
    }

    /// Compute the SHA-256 checksum of a file and return it as a hex string.
    fn compute_file_checksum(path: &Path) -> Result<String, StorageError> {
        use std::io::Read;

        let mut file = std::fs::File::open(path).map_err(|e| {
            StorageError::BackupFailed(format!("failed to open file {}: {}", path.display(), e))
        })?;

        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];
        loop {
            let bytes_read = file.read(&mut buffer).map_err(|e| {
                StorageError::BackupFailed(format!(
                    "failed to read file {}: {}",
                    path.display(),
                    e
                ))
            })?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        let hash = hasher.finalize();
        Ok(hash.iter().map(|b| format!("{:02x}", b)).collect())
    }

    /// Export database using SQLite VACUUM INTO, compute SHA-256 checksum,
    /// and store a record in backup_history table.
    pub async fn backup(&self, dest: &Path) -> Result<(), StorageError> {
        // Ensure destination parent directory exists
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                StorageError::BackupFailed(format!(
                    "failed to create backup directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        // Remove destination file if it already exists (VACUUM INTO fails otherwise)
        if dest.exists() {
            std::fs::remove_file(dest).map_err(|e| {
                StorageError::BackupFailed(format!(
                    "failed to remove existing backup file {}: {}",
                    dest.display(),
                    e
                ))
            })?;
        }

        // Use VACUUM INTO to create a clean copy of the database
        let dest_str = dest.to_string_lossy().replace('\'', "''");
        let vacuum_sql = format!("VACUUM INTO '{}'", dest_str);
        sqlx::raw_sql(&vacuum_sql)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::BackupFailed(format!("VACUUM INTO failed: {}", e)))?;

        // Compute SHA-256 checksum of the backup file
        let checksum = Self::compute_file_checksum(dest)?;

        // Get file size
        let metadata = std::fs::metadata(dest).map_err(|e| {
            StorageError::BackupFailed(format!(
                "failed to read backup metadata {}: {}",
                dest.display(),
                e
            ))
        })?;
        let size_bytes = metadata.len() as i64;

        // Store record in backup_history
        let id = Uuid::new_v4().to_string();
        let created_at = Utc::now().to_rfc3339();
        let file_path = dest.to_string_lossy().to_string();

        sqlx::query(
            "INSERT INTO backup_history (id, created_at, file_path, size_bytes, checksum, description) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&created_at)
        .bind(&file_path)
        .bind(size_bytes)
        .bind(&checksum)
        .bind("automatic backup")
        .execute(&self.pool)
        .await
        .map_err(|e| {
            StorageError::BackupFailed(format!("failed to store backup record: {}", e))
        })?;

        Ok(())
    }

    /// Restore database from a backup file.
    /// Verifies checksum before applying, backs up the current database first for safety,
    /// then replaces the current database with the backup.
    pub async fn restore(&self, src: &Path) -> Result<(), StorageError> {
        // Verify source file exists
        if !src.exists() {
            return Err(StorageError::RestoreFailed(format!(
                "backup file does not exist: {}",
                src.display()
            )));
        }

        // Compute SHA-256 checksum of the source file
        let actual_checksum = Self::compute_file_checksum(src)?;

        // Look up the expected checksum from backup_history by file path
        let src_path_str = src.to_string_lossy().to_string();
        let row = sqlx::query(
            "SELECT checksum FROM backup_history WHERE file_path = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&src_path_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            StorageError::RestoreFailed(format!("failed to query backup history: {}", e))
        })?;

        if let Some(row) = row {
            let expected_checksum: String = row.get("checksum");
            if actual_checksum != expected_checksum {
                return Err(StorageError::ChecksumMismatch(
                    expected_checksum,
                    actual_checksum,
                ));
            }
        }

        // Backup current database first (safety)
        let safety_backup_path = self.db_path.with_extension("pre-restore.bak");
        self.backup(&safety_backup_path).await.map_err(|e| {
            StorageError::RestoreFailed(format!("failed to create safety backup: {:?}", e))
        })?;

        // Force WAL checkpoint to flush all data to the main database file
        // This ensures the WAL is empty before we close the pool and replace the file
        sqlx::raw_sql("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                StorageError::RestoreFailed(format!("failed to checkpoint WAL: {}", e))
            })?;

        // Close current pool
        self.pool.close().await;

        // Remove WAL and SHM files before copying (they're stale after replacement)
        let wal_path = self.db_path.with_extension("db-wal");
        let shm_path = self.db_path.with_extension("db-shm");
        let _ = std::fs::remove_file(&wal_path);
        let _ = std::fs::remove_file(&shm_path);

        // Copy the backup file over the current database
        std::fs::copy(src, &self.db_path).map_err(|e| {
            StorageError::RestoreFailed(format!(
                "failed to copy backup file to {}: {}",
                self.db_path.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Rollback to the most recent valid backup from backup_history.
    /// Finds the most recent backup, verifies it still exists and its checksum matches,
    /// then restores from it.
    pub async fn rollback(&self) -> Result<(), StorageError> {
        // Query the most recent backup entry
        let row = sqlx::query(
            "SELECT file_path, checksum FROM backup_history ORDER BY created_at DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            StorageError::RestoreFailed(format!("failed to query backup history: {}", e))
        })?;

        let row = row.ok_or_else(|| {
            StorageError::RestoreFailed("no backup found in backup_history".to_string())
        })?;

        let file_path: String = row.get("file_path");
        let expected_checksum: String = row.get("checksum");

        let backup_path = std::path::PathBuf::from(&file_path);

        // Verify the backup file still exists
        if !backup_path.exists() {
            return Err(StorageError::RestoreFailed(format!(
                "backup file no longer exists: {}",
                file_path
            )));
        }

        // Verify checksum matches
        let actual_checksum = Self::compute_file_checksum(&backup_path)?;
        if actual_checksum != expected_checksum {
            return Err(StorageError::ChecksumMismatch(
                expected_checksum,
                actual_checksum,
            ));
        }

        // Restore from it
        self.restore(&backup_path).await
    }
}

/// SQL migration statements for all tables.
const MIGRATION_SQL: &str = r#"
-- Core usage events table
CREATE TABLE IF NOT EXISTS usage_events (
    id TEXT PRIMARY KEY,
    fingerprint BLOB NOT NULL,
    provider_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    timestamp_utc TEXT NOT NULL,
    model TEXT,
    input_tokens INTEGER,
    cached_input_tokens INTEGER,
    output_tokens INTEGER,
    reasoning_tokens INTEGER,
    total_tokens INTEGER,
    context_window INTEGER,
    quota_fast_pct REAL,
    quota_standard_pct REAL,
    quota_excess_pct REAL,
    quota_daily_tokens INTEGER,
    session_hash TEXT,
    project_hash TEXT,
    source_count INTEGER DEFAULT 1,
    reconciled_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_events_provider_time ON usage_events(provider_id, timestamp_utc);
CREATE INDEX IF NOT EXISTS idx_events_timestamp ON usage_events(timestamp_utc);
CREATE INDEX IF NOT EXISTS idx_events_fingerprint ON usage_events(fingerprint);
CREATE INDEX IF NOT EXISTS idx_events_model ON usage_events(model);

-- Deduplication tracking
CREATE TABLE IF NOT EXISTS seen_fingerprints (
    fingerprint BLOB PRIMARY KEY,
    first_seen_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Provider collection state
CREATE TABLE IF NOT EXISTS collection_checkpoints (
    provider_id TEXT PRIMARY KEY,
    last_checkpoint TEXT NOT NULL,
    last_success_at TEXT NOT NULL,
    files_processed INTEGER DEFAULT 0,
    events_collected INTEGER DEFAULT 0,
    errors_count INTEGER DEFAULT 0,
    metadata TEXT
);

-- File read positions for incremental collection
CREATE TABLE IF NOT EXISTS file_positions (
    provider_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    byte_offset INTEGER NOT NULL DEFAULT 0,
    last_read_at TEXT NOT NULL,
    PRIMARY KEY (provider_id, file_path)
);

-- Application settings key-value store
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Backup history for restore verification
CREATE TABLE IF NOT EXISTS backup_history (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    file_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    checksum TEXT NOT NULL,
    description TEXT
);
"#;


#[cfg(test)]
mod prop_tests_storage_roundtrip {
    use super::*;
    use crate::types::{EventFingerprint, EventType, QuotaUsage, ReconciledEvent, TokenUsage};
    use chrono::{DateTime, Utc};
    use proptest::prelude::*;
    use tempfile::TempDir;
    use uuid::Uuid;

    /// **Validates: Requirements 4.6, 4.7, 15.1, 15.3, 15.6**

    // ─── Strategies ─────────────────────────────────────────────────────────────

    fn arb_event_type() -> impl Strategy<Value = EventType> {
        prop_oneof![
            Just(EventType::TokenCount),
            Just(EventType::SessionSummary),
            Just(EventType::QuotaSample),
            Just(EventType::DailyAggregate),
        ]
    }

    fn arb_provider_id() -> impl Strategy<Value = String> {
        prop_oneof![Just("codex".to_string()), Just("claude".to_string()),]
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
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0u64..10_000_000),
        )
            .prop_map(|(fast, standard, excess, daily)| QuotaUsage {
                fast_hours_pct: fast,
                standard_pct: standard,
                excess_pct: excess,
                daily_tokens: daily,
            })
    }

    fn arb_timestamp() -> impl Strategy<Value = DateTime<Utc>> {
        let min_ts = chrono::NaiveDate::from_ymd_opt(2023, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let max_ts = chrono::NaiveDate::from_ymd_opt(2025, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();

        (min_ts..max_ts).prop_map(|ts| DateTime::from_timestamp(ts, 0).unwrap())
    }

    fn arb_fingerprint() -> impl Strategy<Value = EventFingerprint> {
        proptest::collection::vec(any::<u8>(), 32).prop_map(|bytes| {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            EventFingerprint::new(arr)
        })
    }

    fn arb_reconciled_event() -> impl Strategy<Value = ReconciledEvent> {
        (
            arb_fingerprint(),
            arb_provider_id(),
            arb_event_type(),
            arb_timestamp(),
            proptest::option::of("[a-z0-9\\-]{3,20}"),
            arb_token_usage(),
            proptest::option::of(1024u64..200_000),
            proptest::option::of(arb_quota_usage()),
            proptest::option::of("[a-f0-9]{16,32}"),
            proptest::option::of("[a-f0-9]{16,32}"),
            (1u8..5),
            arb_timestamp(),
        )
            .prop_map(
                |(
                    fingerprint,
                    provider_id,
                    event_type,
                    timestamp,
                    model,
                    tokens,
                    context_window,
                    quota,
                    session_hash,
                    project_hash,
                    source_count,
                    reconciled_at,
                )| {
                    ReconciledEvent {
                        id: Uuid::new_v4(),
                        fingerprint,
                        provider_id,
                        event_type,
                        timestamp,
                        model,
                        tokens,
                        context_window,
                        quota,
                        session_hash,
                        project_hash,
                        reconciled_at,
                        source_count,
                    }
                },
            )
    }

    fn arb_events_vec(min: usize, max: usize) -> impl Strategy<Value = Vec<ReconciledEvent>> {
        proptest::collection::vec(arb_reconciled_event(), min..=max)
    }

    // ─── Property Tests ─────────────────────────────────────────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(10))]

        /// Property 1: Store then query - events stored with store_events() can be retrieved.
        /// The count of stored events matches what was inserted.
        #[test]
        fn prop_store_then_query(events in arb_events_vec(1, 20)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                let count = storage.store_events(&events).await.unwrap();
                prop_assert_eq!(count as usize, events.len());

                // Verify by querying the total count from the database
                let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events")
                    .fetch_one(storage.pool())
                    .await
                    .unwrap();
                let db_count: i64 = row.get("cnt");
                prop_assert_eq!(db_count as usize, events.len());

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 2: Backup creates valid file - after backup(), a file exists at
        /// the destination path with non-zero size.
        #[test]
        fn prop_backup_creates_valid_file(events in arb_events_vec(1, 10)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test.db");
                let backup_path = tmp_dir.path().join("backup.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                storage.store_events(&events).await.unwrap();
                storage.backup(&backup_path).await.unwrap();

                // Verify backup file exists and has non-zero size
                prop_assert!(backup_path.exists(), "Backup file should exist");
                let metadata = std::fs::metadata(&backup_path).unwrap();
                prop_assert!(metadata.len() > 0, "Backup file should have non-zero size");

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 3: Backup checksum is deterministic - running backup() produces
        /// a file whose computed SHA-256 matches what's recorded in backup_history.
        #[test]
        fn prop_backup_checksum_matches_history(events in arb_events_vec(1, 10)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test.db");
                let backup_path = tmp_dir.path().join("backup.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                storage.store_events(&events).await.unwrap();
                storage.backup(&backup_path).await.unwrap();

                // Compute checksum of backup file independently
                let actual_checksum = StorageLayer::compute_file_checksum(&backup_path).unwrap();

                // Query checksum from backup_history
                let row = sqlx::query(
                    "SELECT checksum FROM backup_history ORDER BY created_at DESC LIMIT 1"
                )
                .fetch_one(storage.pool())
                .await
                .unwrap();
                let stored_checksum: String = row.get("checksum");

                prop_assert_eq!(
                    actual_checksum, stored_checksum,
                    "Computed checksum must match stored checksum in backup_history"
                );

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 4: Restore after backup preserves data - after backup → store more
        /// data → restore, the database should contain only the data present at backup time.
        /// Original events should be present, new events should be gone.
        #[test]
        fn prop_restore_preserves_backup_data(
            original_events in arb_events_vec(1, 10),
            extra_events in arb_events_vec(1, 5),
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test.db");
                let backup_path = tmp_dir.path().join("backup.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                // Store original events and backup
                storage.store_events(&original_events).await.unwrap();
                storage.backup(&backup_path).await.unwrap();

                // Store extra events after backup
                storage.store_events(&extra_events).await.unwrap();

                // Collect original and extra event IDs for verification
                let original_ids: Vec<String> = original_events.iter()
                    .map(|e| e.id.to_string())
                    .collect();
                let extra_ids: Vec<String> = extra_events.iter()
                    .map(|e| e.id.to_string())
                    .collect();

                // Restore from backup
                storage.restore(&backup_path).await.unwrap();

                // After restore, re-open the database to read the restored state
                let storage2 = StorageLayer::new(&db_path).await.unwrap();

                // Verify original events ARE present
                for oid in &original_ids {
                    let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events WHERE id = ?")
                        .bind(oid)
                        .fetch_one(storage2.pool())
                        .await
                        .unwrap();
                    let found: i64 = row.get("cnt");
                    prop_assert_eq!(found, 1,
                        "Original event {} should be present after restore", oid);
                }

                // Verify extra events are NOT present
                for eid in &extra_ids {
                    let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events WHERE id = ?")
                        .bind(eid)
                        .fetch_one(storage2.pool())
                        .await
                        .unwrap();
                    let found: i64 = row.get("cnt");
                    prop_assert_eq!(found, 0,
                        "Extra event {} should NOT be present after restore", eid);
                }

                storage2.close().await;
                Ok(())
            })?;
        }

        /// Property 5: Rollback finds most recent backup - after multiple backups,
        /// rollback() uses the most recent valid one.
        #[test]
        fn prop_rollback_uses_most_recent_backup(
            events_batch1 in arb_events_vec(1, 5),
            events_batch2 in arb_events_vec(1, 5),
            events_batch3 in arb_events_vec(1, 5),
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test.db");
                let backup1_path = tmp_dir.path().join("backup1.db");
                let backup2_path = tmp_dir.path().join("backup2.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                // Store batch1 and create first backup
                storage.store_events(&events_batch1).await.unwrap();
                storage.backup(&backup1_path).await.unwrap();

                // Store batch2 and create second backup (more recent)
                storage.store_events(&events_batch2).await.unwrap();
                storage.backup(&backup2_path).await.unwrap();

                // Store batch3 (after all backups)
                storage.store_events(&events_batch3).await.unwrap();

                // Collect IDs for verification
                let batch1_ids: Vec<String> = events_batch1.iter()
                    .map(|e| e.id.to_string())
                    .collect();
                let batch2_ids: Vec<String> = events_batch2.iter()
                    .map(|e| e.id.to_string())
                    .collect();
                let batch3_ids: Vec<String> = events_batch3.iter()
                    .map(|e| e.id.to_string())
                    .collect();

                // Rollback should use the most recent backup (backup2)
                storage.rollback().await.unwrap();

                // Re-open and verify state matches backup2 (batch1 + batch2)
                let storage2 = StorageLayer::new(&db_path).await.unwrap();

                // Batch1 events should be present (they were in backup2)
                for id in &batch1_ids {
                    let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events WHERE id = ?")
                        .bind(id)
                        .fetch_one(storage2.pool())
                        .await
                        .unwrap();
                    let found: i64 = row.get("cnt");
                    prop_assert_eq!(found, 1,
                        "Batch1 event {} should be present after rollback", id);
                }

                // Batch2 events should be present (they were in backup2)
                for id in &batch2_ids {
                    let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events WHERE id = ?")
                        .bind(id)
                        .fetch_one(storage2.pool())
                        .await
                        .unwrap();
                    let found: i64 = row.get("cnt");
                    prop_assert_eq!(found, 1,
                        "Batch2 event {} should be present after rollback", id);
                }

                // Batch3 events should NOT be present (added after backup2)
                for id in &batch3_ids {
                    let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events WHERE id = ?")
                        .bind(id)
                        .fetch_one(storage2.pool())
                        .await
                        .unwrap();
                    let found: i64 = row.get("cnt");
                    prop_assert_eq!(found, 0,
                        "Batch3 event {} should NOT be present after rollback", id);
                }

                storage2.close().await;
                Ok(())
            })?;
        }
    }
}


#[cfg(test)]
mod prop_tests_backup_corruption {
    use super::*;
    use crate::error::StorageError;
    use crate::types::{EventFingerprint, EventType, QuotaUsage, ReconciledEvent, TokenUsage};
    use chrono::{DateTime, Utc};
    use proptest::prelude::*;
    use tempfile::TempDir;
    use uuid::Uuid;

    /// **Validates: Requirements 15.2, 15.4**

    // ─── Strategies ─────────────────────────────────────────────────────────────

    fn arb_event_type() -> impl Strategy<Value = EventType> {
        prop_oneof![
            Just(EventType::TokenCount),
            Just(EventType::SessionSummary),
            Just(EventType::QuotaSample),
            Just(EventType::DailyAggregate),
        ]
    }

    fn arb_provider_id() -> impl Strategy<Value = String> {
        prop_oneof![Just("codex".to_string()), Just("claude".to_string()),]
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
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0u64..10_000_000),
        )
            .prop_map(|(fast, standard, excess, daily)| QuotaUsage {
                fast_hours_pct: fast,
                standard_pct: standard,
                excess_pct: excess,
                daily_tokens: daily,
            })
    }

    fn arb_timestamp() -> impl Strategy<Value = DateTime<Utc>> {
        let min_ts = chrono::NaiveDate::from_ymd_opt(2023, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let max_ts = chrono::NaiveDate::from_ymd_opt(2025, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();

        (min_ts..max_ts).prop_map(|ts| DateTime::from_timestamp(ts, 0).unwrap())
    }

    fn arb_fingerprint() -> impl Strategy<Value = EventFingerprint> {
        proptest::collection::vec(any::<u8>(), 32).prop_map(|bytes| {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            EventFingerprint::new(arr)
        })
    }

    fn arb_reconciled_event() -> impl Strategy<Value = ReconciledEvent> {
        (
            arb_fingerprint(),
            arb_provider_id(),
            arb_event_type(),
            arb_timestamp(),
            proptest::option::of("[a-z0-9\\-]{3,20}"),
            arb_token_usage(),
            proptest::option::of(1024u64..200_000),
            proptest::option::of(arb_quota_usage()),
            proptest::option::of("[a-f0-9]{16,32}"),
            proptest::option::of("[a-f0-9]{16,32}"),
            (1u8..5),
            arb_timestamp(),
        )
            .prop_map(
                |(
                    fingerprint,
                    provider_id,
                    event_type,
                    timestamp,
                    model,
                    tokens,
                    context_window,
                    quota,
                    session_hash,
                    project_hash,
                    source_count,
                    reconciled_at,
                )| {
                    ReconciledEvent {
                        id: Uuid::new_v4(),
                        fingerprint,
                        provider_id,
                        event_type,
                        timestamp,
                        model,
                        tokens,
                        context_window,
                        quota,
                        session_hash,
                        project_hash,
                        reconciled_at,
                        source_count,
                    }
                },
            )
    }

    fn arb_events_vec(min: usize, max: usize) -> impl Strategy<Value = Vec<ReconciledEvent>> {
        proptest::collection::vec(arb_reconciled_event(), min..=max)
    }

    // ─── Property Tests ─────────────────────────────────────────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(10))]

        /// Property 1: Corrupted backup rejected - If a backup file's content is modified
        /// (any byte changed), restore() should return a ChecksumMismatch error. (Req 15.4)
        #[test]
        fn prop_corrupted_backup_rejected(
            events in arb_events_vec(1, 10),
            flip_offset_pct in 1u8..100u8,
            flip_value in 1u8..=255u8,
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test.db");
                let backup_path = tmp_dir.path().join("backup.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                // Store events and create a backup
                storage.store_events(&events).await.unwrap();
                storage.backup(&backup_path).await.unwrap();

                // Read the backup file and corrupt it by flipping a byte
                let mut data = std::fs::read(&backup_path).unwrap();
                prop_assert!(!data.is_empty(), "Backup file should not be empty");

                let offset = (flip_offset_pct as usize * (data.len() - 1)) / 100;
                // XOR with a non-zero value to guarantee the byte changes
                data[offset] ^= flip_value;
                std::fs::write(&backup_path, &data).unwrap();

                // Attempt to restore - should fail with ChecksumMismatch
                let result = storage.restore(&backup_path).await;
                match result {
                    Err(StorageError::ChecksumMismatch(expected, actual)) => {
                        prop_assert_ne!(
                            expected, actual,
                            "Expected and actual checksums must differ for corrupted file"
                        );
                    }
                    other => {
                        prop_assert!(
                            false,
                            "Expected ChecksumMismatch error, got: {:?}",
                            other
                        );
                    }
                }

                Ok(())
            })?;
        }

        /// Property 2: Truncated backup rejected - If a backup file is truncated
        /// (shorter than original), restore() should detect the mismatch. (Req 15.2)
        #[test]
        fn prop_truncated_backup_rejected(
            events in arb_events_vec(1, 10),
            truncate_pct in 10u8..90u8,
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test.db");
                let backup_path = tmp_dir.path().join("backup.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                // Store events and create a backup
                storage.store_events(&events).await.unwrap();
                storage.backup(&backup_path).await.unwrap();

                // Truncate the backup file
                let data = std::fs::read(&backup_path).unwrap();
                prop_assert!(data.len() > 1, "Backup file must be larger than 1 byte");

                let new_len = (truncate_pct as usize * data.len()) / 100;
                let new_len = new_len.max(1); // At least 1 byte
                std::fs::write(&backup_path, &data[..new_len]).unwrap();

                // Attempt to restore - should fail with ChecksumMismatch
                let result = storage.restore(&backup_path).await;
                match result {
                    Err(StorageError::ChecksumMismatch(expected, actual)) => {
                        prop_assert_ne!(
                            expected, actual,
                            "Expected and actual checksums must differ for truncated file"
                        );
                    }
                    other => {
                        prop_assert!(
                            false,
                            "Expected ChecksumMismatch error, got: {:?}",
                            other
                        );
                    }
                }

                Ok(())
            })?;
        }

        /// Property 3: Appended data detected - If extra bytes are appended to a backup
        /// file, the checksum changes and restore rejects it. (Req 15.2)
        #[test]
        fn prop_appended_data_detected(
            events in arb_events_vec(1, 10),
            extra_bytes in proptest::collection::vec(any::<u8>(), 1..100),
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test.db");
                let backup_path = tmp_dir.path().join("backup.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                // Store events and create a backup
                storage.store_events(&events).await.unwrap();
                storage.backup(&backup_path).await.unwrap();

                // Append extra bytes to the backup file
                let mut data = std::fs::read(&backup_path).unwrap();
                data.extend_from_slice(&extra_bytes);
                std::fs::write(&backup_path, &data).unwrap();

                // Attempt to restore - should fail with ChecksumMismatch
                let result = storage.restore(&backup_path).await;
                match result {
                    Err(StorageError::ChecksumMismatch(expected, actual)) => {
                        prop_assert_ne!(
                            expected, actual,
                            "Expected and actual checksums must differ for file with appended data"
                        );
                    }
                    other => {
                        prop_assert!(
                            false,
                            "Expected ChecksumMismatch error, got: {:?}",
                            other
                        );
                    }
                }

                Ok(())
            })?;
        }

        /// Property 4: Valid backup accepted - An unmodified backup file passes
        /// checksum verification and restore succeeds. (Baseline)
        #[test]
        fn prop_valid_backup_accepted(events in arb_events_vec(1, 10)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test.db");
                let backup_path = tmp_dir.path().join("backup.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                // Store events and create a backup
                storage.store_events(&events).await.unwrap();
                storage.backup(&backup_path).await.unwrap();

                // Restore without modifying the backup - should succeed
                let result = storage.restore(&backup_path).await;
                prop_assert!(
                    result.is_ok(),
                    "Restore of unmodified backup should succeed, got: {:?}",
                    result
                );

                Ok(())
            })?;
        }

        /// Property 5: Checksum stored correctly - The checksum in backup_history
        /// matches the actual file's SHA-256. (Req 15.2)
        #[test]
        fn prop_checksum_stored_correctly(events in arb_events_vec(1, 10)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test.db");
                let backup_path = tmp_dir.path().join("backup.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                // Store events and create a backup
                storage.store_events(&events).await.unwrap();
                storage.backup(&backup_path).await.unwrap();

                // Compute SHA-256 of the backup file independently
                let actual_checksum = StorageLayer::compute_file_checksum(&backup_path).unwrap();

                // Query checksum from backup_history
                let row = sqlx::query(
                    "SELECT checksum FROM backup_history ORDER BY created_at DESC LIMIT 1",
                )
                .fetch_one(storage.pool())
                .await
                .unwrap();
                let stored_checksum: String = row.get("checksum");

                prop_assert_eq!(
                    actual_checksum,
                    stored_checksum,
                    "SHA-256 of backup file must match checksum stored in backup_history"
                );

                storage.close().await;
                Ok(())
            })?;
        }
    }
}


#[cfg(test)]
mod prop_tests_retention {
    use super::*;
    use crate::types::{EventFingerprint, EventType, QuotaUsage, ReconciledEvent, TokenUsage};
    use chrono::{DateTime, Duration, Utc};
    use proptest::prelude::*;
    use tempfile::TempDir;
    use uuid::Uuid;

    /// **Validates: Requirements 4.3**

    // ─── Strategies ─────────────────────────────────────────────────────────────

    fn arb_event_type() -> impl Strategy<Value = EventType> {
        prop_oneof![
            Just(EventType::TokenCount),
            Just(EventType::SessionSummary),
            Just(EventType::QuotaSample),
            Just(EventType::DailyAggregate),
        ]
    }

    fn arb_provider_id() -> impl Strategy<Value = String> {
        prop_oneof![Just("codex".to_string()), Just("claude".to_string()),]
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
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0u64..10_000_000),
        )
            .prop_map(|(fast, standard, excess, daily)| QuotaUsage {
                fast_hours_pct: fast,
                standard_pct: standard,
                excess_pct: excess,
                daily_tokens: daily,
            })
    }

    fn arb_fingerprint() -> impl Strategy<Value = EventFingerprint> {
        proptest::collection::vec(any::<u8>(), 32).prop_map(|bytes| {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            EventFingerprint::new(arr)
        })
    }

    /// Create a reconciled event with a specific timestamp.
    fn make_event_at(timestamp: DateTime<Utc>) -> impl Strategy<Value = ReconciledEvent> {
        (
            arb_fingerprint(),
            arb_provider_id(),
            arb_event_type(),
            proptest::option::of("[a-z0-9\\-]{3,20}"),
            arb_token_usage(),
            proptest::option::of(1024u64..200_000),
            proptest::option::of(arb_quota_usage()),
            proptest::option::of("[a-f0-9]{16,32}"),
            proptest::option::of("[a-f0-9]{16,32}"),
            (1u8..5),
        )
            .prop_map(
                move |(
                    fingerprint,
                    provider_id,
                    event_type,
                    model,
                    tokens,
                    context_window,
                    quota,
                    session_hash,
                    project_hash,
                    source_count,
                )| {
                    ReconciledEvent {
                        id: Uuid::new_v4(),
                        fingerprint,
                        provider_id,
                        event_type,
                        timestamp,
                        model,
                        tokens,
                        context_window,
                        quota,
                        session_hash,
                        project_hash,
                        reconciled_at: Utc::now(),
                        source_count,
                    }
                },
            )
    }

    /// Generate a mix of old events (before cutoff) and recent events (after cutoff)
    /// for a given retention_days.
    fn arb_mixed_events(
        retention_days: u32,
    ) -> impl Strategy<Value = (Vec<ReconciledEvent>, Vec<ReconciledEvent>)> {
        let now = Utc::now();
        // Old: between 2x retention and retention+1 days ago
        let old_ts_max = (now - Duration::days(retention_days as i64 + 1)).timestamp();
        let old_ts_min = (now - Duration::days(retention_days as i64 * 2)).timestamp();
        // Recent: between now and retention_days/2 ago (well within retention)
        let recent_ts_min = (now - Duration::days(retention_days as i64 / 2)).timestamp();
        let recent_ts_max = now.timestamp();

        let old_events = proptest::collection::vec(
            (old_ts_min..old_ts_max).prop_flat_map(|ts| {
                let timestamp = DateTime::from_timestamp(ts, 0).unwrap();
                make_event_at(timestamp)
            }),
            1..=5,
        );

        let recent_events = proptest::collection::vec(
            (recent_ts_min..recent_ts_max).prop_flat_map(|ts| {
                let timestamp = DateTime::from_timestamp(ts, 0).unwrap();
                make_event_at(timestamp)
            }),
            1..=5,
        );

        (old_events, recent_events)
    }

    // ─── Property Tests ─────────────────────────────────────────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(10))]

        /// Property 1: Old events pruned - Events with timestamps older than
        /// retention_days from now are deleted by prune_expired(). (Req 4.3)
        #[test]
        fn prop_old_events_pruned(
            (old_events, recent_events) in arb_mixed_events(30),
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_retention.db");
                let storage = StorageLayer::with_retention(&db_path, 30).await.unwrap();

                // Store old and recent events
                storage.store_events(&old_events).await.unwrap();
                storage.store_events(&recent_events).await.unwrap();

                // Prune expired events
                storage.prune_expired().await.unwrap();

                // Verify old events are gone
                for event in &old_events {
                    let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events WHERE id = ?")
                        .bind(event.id.to_string())
                        .fetch_one(storage.pool())
                        .await
                        .unwrap();
                    let found: i64 = row.get("cnt");
                    prop_assert_eq!(found, 0,
                        "Old event {} with timestamp {} should have been pruned (retention=30 days)",
                        event.id, event.timestamp);
                }

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 2: Recent events preserved - Events with timestamps within the
        /// retention period remain after prune_expired().
        #[test]
        fn prop_recent_events_preserved(
            (old_events, recent_events) in arb_mixed_events(30),
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_retention.db");
                let storage = StorageLayer::with_retention(&db_path, 30).await.unwrap();

                // Store old and recent events
                storage.store_events(&old_events).await.unwrap();
                storage.store_events(&recent_events).await.unwrap();

                // Prune expired events
                storage.prune_expired().await.unwrap();

                // Verify recent events are still present
                for event in &recent_events {
                    let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events WHERE id = ?")
                        .bind(event.id.to_string())
                        .fetch_one(storage.pool())
                        .await
                        .unwrap();
                    let found: i64 = row.get("cnt");
                    prop_assert_eq!(found, 1,
                        "Recent event {} with timestamp {} should be preserved (retention=30 days)",
                        event.id, event.timestamp);
                }

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 3: Exact boundary - Events exactly at the boundary (retention_days ago)
        /// should be pruned (< cutoff, not <=). The prune_expired method uses strict less-than.
        #[test]
        fn prop_exact_boundary_pruned(
            event_base in make_event_at(Utc::now()), // dummy, we override timestamp
            retention_days in 7u32..60,
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_boundary.db");
                let storage = StorageLayer::with_retention(&db_path, retention_days).await.unwrap();

                // Create an event exactly at the cutoff boundary
                // The cutoff is: now - retention_days
                // The query is: DELETE WHERE timestamp_utc < cutoff
                // An event exactly at cutoff should NOT be pruned (it's not < cutoff, it IS cutoff)
                // But let's test an event 1 second before cutoff (should be pruned)
                let cutoff = Utc::now() - Duration::days(retention_days as i64);
                let just_before_cutoff = cutoff - Duration::seconds(1);

                let mut boundary_event = event_base.clone();
                boundary_event.id = Uuid::new_v4();
                boundary_event.timestamp = just_before_cutoff;

                // Also create an event 1 second after cutoff (should be preserved)
                let just_after_cutoff = cutoff + Duration::seconds(1);
                let mut safe_event = event_base;
                safe_event.id = Uuid::new_v4();
                safe_event.timestamp = just_after_cutoff;

                storage.store_events(&[boundary_event.clone(), safe_event.clone()]).await.unwrap();

                // Prune
                storage.prune_expired().await.unwrap();

                // Event just before cutoff should be pruned
                let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events WHERE id = ?")
                    .bind(boundary_event.id.to_string())
                    .fetch_one(storage.pool())
                    .await
                    .unwrap();
                let found: i64 = row.get("cnt");
                prop_assert_eq!(found, 0,
                    "Event 1s before cutoff should be pruned");

                // Event just after cutoff should be preserved
                let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events WHERE id = ?")
                    .bind(safe_event.id.to_string())
                    .fetch_one(storage.pool())
                    .await
                    .unwrap();
                let found: i64 = row.get("cnt");
                prop_assert_eq!(found, 1,
                    "Event 1s after cutoff should be preserved");

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 4: Prune count correct - prune_expired() returns the exact number
        /// of events deleted.
        #[test]
        fn prop_prune_count_correct(
            (old_events, recent_events) in arb_mixed_events(30),
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_prune_count.db");
                let storage = StorageLayer::with_retention(&db_path, 30).await.unwrap();

                // Store old and recent events
                storage.store_events(&old_events).await.unwrap();
                storage.store_events(&recent_events).await.unwrap();

                let total_before = old_events.len() + recent_events.len();

                // Prune expired events
                let pruned_count = storage.prune_expired().await.unwrap();

                // The pruned count should equal the number of old events
                prop_assert_eq!(pruned_count as usize, old_events.len(),
                    "prune_expired() should return {} (old events count), got {}",
                    old_events.len(), pruned_count);

                // Verify remaining count
                let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events")
                    .fetch_one(storage.pool())
                    .await
                    .unwrap();
                let remaining: i64 = row.get("cnt");
                prop_assert_eq!(remaining as usize, recent_events.len(),
                    "After pruning, {} events should remain (recent), got {}",
                    recent_events.len(), remaining);

                // Confirm: total_before - pruned = remaining
                prop_assert_eq!(
                    total_before - pruned_count as usize,
                    remaining as usize,
                    "total_before ({}) - pruned ({}) should equal remaining ({})",
                    total_before, pruned_count, remaining
                );

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 5: Configurable retention - StorageLayer::with_retention() with different
        /// retention_days properly changes the cutoff.
        #[test]
        fn prop_configurable_retention(
            retention_days in 7u32..90,
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_configurable.db");
                let storage = StorageLayer::with_retention(&db_path, retention_days).await.unwrap();

                let now = Utc::now();

                // Create an event that is retention_days + 10 days old (should be pruned)
                let old_timestamp = now - Duration::days(retention_days as i64 + 10);
                // Create an event that is retention_days - 5 days old (should be preserved)
                let recent_timestamp = now - Duration::days(retention_days as i64 - 5);

                let old_event = ReconciledEvent {
                    id: Uuid::new_v4(),
                    fingerprint: EventFingerprint::new([1u8; 32]),
                    provider_id: "codex".to_string(),
                    event_type: EventType::TokenCount,
                    timestamp: old_timestamp,
                    model: Some("gpt-4".to_string()),
                    tokens: TokenUsage::default(),
                    context_window: None,
                    quota: None,
                    session_hash: None,
                    project_hash: None,
                    reconciled_at: now,
                    source_count: 1,
                };

                let recent_event = ReconciledEvent {
                    id: Uuid::new_v4(),
                    fingerprint: EventFingerprint::new([2u8; 32]),
                    provider_id: "claude".to_string(),
                    event_type: EventType::TokenCount,
                    timestamp: recent_timestamp,
                    model: Some("claude-3".to_string()),
                    tokens: TokenUsage::default(),
                    context_window: None,
                    quota: None,
                    session_hash: None,
                    project_hash: None,
                    reconciled_at: now,
                    source_count: 1,
                };

                storage.store_events(&[old_event.clone(), recent_event.clone()]).await.unwrap();

                // Verify retention_days is set correctly
                prop_assert_eq!(storage.retention_days, retention_days);

                // Prune
                let pruned = storage.prune_expired().await.unwrap();
                prop_assert_eq!(pruned, 1,
                    "Should prune exactly 1 old event with retention_days={}", retention_days);

                // Verify old event is gone, recent is preserved
                let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events WHERE id = ?")
                    .bind(old_event.id.to_string())
                    .fetch_one(storage.pool())
                    .await
                    .unwrap();
                let old_found: i64 = row.get("cnt");
                prop_assert_eq!(old_found, 0, "Old event should be pruned");

                let row = sqlx::query("SELECT COUNT(*) as cnt FROM usage_events WHERE id = ?")
                    .bind(recent_event.id.to_string())
                    .fetch_one(storage.pool())
                    .await
                    .unwrap();
                let recent_found: i64 = row.get("cnt");
                prop_assert_eq!(recent_found, 1, "Recent event should be preserved");

                storage.close().await;
                Ok(())
            })?;
        }
    }
}


#[cfg(test)]
mod prop_tests_aggregation {
    use super::*;
    use crate::query_types::{Granularity, TimeRange};
    use crate::types::{EventFingerprint, EventType, ReconciledEvent, TokenUsage};
    use chrono::{DateTime, Duration, Timelike, Utc};
    use proptest::prelude::*;
    use tempfile::TempDir;
    use uuid::Uuid;

    /// **Validates: Requirements 4.4, 7.5**

    // ─── Strategies ─────────────────────────────────────────────────────────────

    fn arb_event_type() -> impl Strategy<Value = EventType> {
        prop_oneof![
            Just(EventType::TokenCount),
            Just(EventType::SessionSummary),
            Just(EventType::QuotaSample),
            Just(EventType::DailyAggregate),
        ]
    }

    fn arb_provider_id() -> impl Strategy<Value = String> {
        prop_oneof![Just("codex".to_string()), Just("claude".to_string()),]
    }

    fn arb_token_usage_with_total() -> impl Strategy<Value = TokenUsage> {
        // Generate input and output, then derive total to ensure consistency
        (1u64..100_000, 1u64..100_000).prop_map(|(input, output)| {
            let total = input + output;
            TokenUsage {
                input_tokens: Some(input),
                cached_input_tokens: None,
                output_tokens: Some(output),
                reasoning_tokens: None,
                total_tokens: Some(total),
            }
        })
    }

    fn arb_fingerprint() -> impl Strategy<Value = EventFingerprint> {
        proptest::collection::vec(any::<u8>(), 32).prop_map(|bytes| {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            EventFingerprint::new(arr)
        })
    }

    /// Generate a reconciled event with a specific timestamp and provider.
    fn arb_event_with_ts_and_provider(
        timestamp: DateTime<Utc>,
        provider_id: String,
    ) -> impl Strategy<Value = ReconciledEvent> {
        (
            arb_fingerprint(),
            arb_event_type(),
            arb_token_usage_with_total(),
        )
            .prop_map(move |(fingerprint, event_type, tokens)| ReconciledEvent {
                id: Uuid::new_v4(),
                fingerprint,
                provider_id: provider_id.clone(),
                event_type,
                timestamp,
                model: Some("test-model".to_string()),
                tokens,
                context_window: None,
                quota: None,
                session_hash: None,
                project_hash: None,
                reconciled_at: Utc::now(),
                source_count: 1,
            })
    }

    /// Generate multiple events for today, distributed across providers.
    fn arb_today_events(count: usize) -> impl Strategy<Value = Vec<ReconciledEvent>> {
        let now = Utc::now();

        // "Today" is the local day (see current_period_starts), so the window
        // is clamped to local midnight — otherwise a run just after midnight
        // generates events that belong to yesterday and the sums disagree.
        let local_midnight = Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|naive| Local.from_local_datetime(&naive).earliest())
            .map(|dt| dt.with_timezone(&Utc).timestamp())
            .unwrap_or_else(|| (now - Duration::days(1)).timestamp());

        let min_ts = (now - Duration::hours(2)).timestamp().max(local_midnight);
        let max_ts = now.timestamp().max(min_ts + 1);

        proptest::collection::vec(
            (min_ts..max_ts, arb_provider_id()).prop_flat_map(|(ts, provider)| {
                let timestamp = DateTime::from_timestamp(ts, 0).unwrap();
                arb_event_with_ts_and_provider(timestamp, provider)
            }),
            1..=count,
        )
    }

    // ─── Property Tests ─────────────────────────────────────────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(10))]

        /// Property 1: Total tokens sum correctly - The total_tokens_today in UsageSummary
        /// equals the sum of all individual events' total_tokens stored today.
        #[test]
        fn prop_total_tokens_sum_correctly(events in arb_today_events(10)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_agg.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                storage.store_events(&events).await.unwrap();

                let summary = storage.get_current_summary().await.unwrap();

                // Compute expected total from individual events
                let expected_total: i64 = events
                    .iter()
                    .filter_map(|e| e.tokens.total_tokens.map(|t| t as i64))
                    .sum();

                let actual_total = summary.total_tokens_today.unwrap_or(0);

                prop_assert_eq!(
                    actual_total, expected_total,
                    "total_tokens_today ({}) should equal sum of all events' total_tokens ({})",
                    actual_total, expected_total
                );

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 2: Per-provider breakdown - Each ProviderUsageSummary correctly
        /// sums only events from that provider.
        #[test]
        fn prop_per_provider_breakdown(events in arb_today_events(10)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_provider_agg.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                storage.store_events(&events).await.unwrap();

                let summary = storage.get_current_summary().await.unwrap();

                // For each provider in the summary, verify the sum matches
                for provider_summary in &summary.providers {
                    let pid = &provider_summary.provider_id;

                    let expected_total: i64 = events
                        .iter()
                        .filter(|e| &e.provider_id == pid)
                        .filter_map(|e| e.tokens.total_tokens.map(|t| t as i64))
                        .sum();

                    let expected_input: i64 = events
                        .iter()
                        .filter(|e| &e.provider_id == pid)
                        .filter_map(|e| e.tokens.input_tokens.map(|t| t as i64))
                        .sum();

                    let expected_output: i64 = events
                        .iter()
                        .filter(|e| &e.provider_id == pid)
                        .filter_map(|e| e.tokens.output_tokens.map(|t| t as i64))
                        .sum();

                    let actual_total = provider_summary.total_tokens_today.unwrap_or(0);
                    let actual_input = provider_summary.input_tokens_today.unwrap_or(0);
                    let actual_output = provider_summary.output_tokens_today.unwrap_or(0);

                    prop_assert_eq!(
                        actual_total, expected_total,
                        "Provider {} total_tokens_today ({}) != expected ({})",
                        pid, actual_total, expected_total
                    );
                    prop_assert_eq!(
                        actual_input, expected_input,
                        "Provider {} input_tokens_today ({}) != expected ({})",
                        pid, actual_input, expected_input
                    );
                    prop_assert_eq!(
                        actual_output, expected_output,
                        "Provider {} output_tokens_today ({}) != expected ({})",
                        pid, actual_output, expected_output
                    );
                }

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 3: History respects time range - get_history() only includes events
        /// within the specified TimeRange (events outside are excluded).
        #[test]
        fn prop_history_respects_time_range(
            inside_offset_secs in 60i64..3500,
            outside_offset_secs in 3700i64..7200,
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_history_range.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                let now = Utc::now();
                // Define a 1-hour window
                let range_start = now - Duration::hours(1);
                let range_end = now;

                // Create an event inside the range
                let inside_ts = range_start + Duration::seconds(inside_offset_secs);
                let inside_event = ReconciledEvent {
                    id: Uuid::new_v4(),
                    fingerprint: EventFingerprint::new([10u8; 32]),
                    provider_id: "codex".to_string(),
                    event_type: EventType::TokenCount,
                    timestamp: inside_ts,
                    model: Some("test-model".to_string()),
                    tokens: TokenUsage {
                        input_tokens: Some(100),
                        cached_input_tokens: None,
                        output_tokens: Some(50),
                        reasoning_tokens: None,
                        total_tokens: Some(150),
                    },
                    context_window: None,
                    quota: None,
                    session_hash: None,
                    project_hash: None,
                    reconciled_at: now,
                    source_count: 1,
                };

                // Create an event outside the range (before range_start)
                let outside_ts = range_start - Duration::seconds(outside_offset_secs);
                let outside_event = ReconciledEvent {
                    id: Uuid::new_v4(),
                    fingerprint: EventFingerprint::new([20u8; 32]),
                    provider_id: "codex".to_string(),
                    event_type: EventType::TokenCount,
                    timestamp: outside_ts,
                    model: Some("test-model".to_string()),
                    tokens: TokenUsage {
                        input_tokens: Some(200),
                        cached_input_tokens: None,
                        output_tokens: Some(100),
                        reasoning_tokens: None,
                        total_tokens: Some(300),
                    },
                    context_window: None,
                    quota: None,
                    session_hash: None,
                    project_hash: None,
                    reconciled_at: now,
                    source_count: 1,
                };

                storage.store_events(&[inside_event, outside_event]).await.unwrap();

                let time_range = TimeRange {
                    start: range_start,
                    end: range_end,
                };
                let records = storage.get_history(&time_range, Granularity::Hourly).await.unwrap();

                // Sum of total_tokens in history should only include the inside event
                let history_total: i64 = records
                    .iter()
                    .filter_map(|r| r.total_tokens)
                    .sum();

                prop_assert_eq!(
                    history_total, 150,
                    "History total ({}) should only include inside event (150), not outside (300)",
                    history_total
                );

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 4: Granularity bucketing - Events at different times within the same
        /// granularity bucket get aggregated together (e.g., two events in the same hour
        /// with Hourly granularity produce one record with summed tokens).
        #[test]
        fn prop_granularity_bucketing(
            tokens_a in 1u64..50_000,
            tokens_b in 1u64..50_000,
            minute_offset in 1u32..58,
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_bucketing.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                let now = Utc::now();
                // Put both events in the same hour bucket (2 hours ago, same hour)
                let base_hour = now - Duration::hours(2);
                let bucket_start = base_hour
                    .with_minute(0).unwrap()
                    .with_second(0).unwrap()
                    .with_nanosecond(0).unwrap();

                let ts_a = bucket_start + Duration::minutes(5);
                let ts_b = bucket_start + Duration::minutes(minute_offset as i64);

                let event_a = ReconciledEvent {
                    id: Uuid::new_v4(),
                    fingerprint: EventFingerprint::new([30u8; 32]),
                    provider_id: "codex".to_string(),
                    event_type: EventType::TokenCount,
                    timestamp: ts_a,
                    model: Some("test-model".to_string()),
                    tokens: TokenUsage {
                        input_tokens: Some(tokens_a),
                        cached_input_tokens: None,
                        output_tokens: Some(tokens_a / 2),
                        reasoning_tokens: None,
                        total_tokens: Some(tokens_a + tokens_a / 2),
                    },
                    context_window: None,
                    quota: None,
                    session_hash: None,
                    project_hash: None,
                    reconciled_at: now,
                    source_count: 1,
                };

                let event_b = ReconciledEvent {
                    id: Uuid::new_v4(),
                    fingerprint: EventFingerprint::new([31u8; 32]),
                    provider_id: "codex".to_string(),
                    event_type: EventType::TokenCount,
                    timestamp: ts_b,
                    model: Some("test-model".to_string()),
                    tokens: TokenUsage {
                        input_tokens: Some(tokens_b),
                        cached_input_tokens: None,
                        output_tokens: Some(tokens_b / 2),
                        reasoning_tokens: None,
                        total_tokens: Some(tokens_b + tokens_b / 2),
                    },
                    context_window: None,
                    quota: None,
                    session_hash: None,
                    project_hash: None,
                    reconciled_at: now,
                    source_count: 1,
                };

                storage.store_events(&[event_a, event_b]).await.unwrap();

                // Query with hourly granularity covering that bucket
                let time_range = TimeRange {
                    start: bucket_start - Duration::minutes(1),
                    end: bucket_start + Duration::hours(1) + Duration::minutes(1),
                };
                let records = storage.get_history(&time_range, Granularity::Hourly).await.unwrap();

                // Both events are in the same hour bucket for the same provider,
                // so there should be exactly 1 record
                let codex_records: Vec<_> = records
                    .iter()
                    .filter(|r| r.provider_id == "codex")
                    .collect();

                prop_assert_eq!(
                    codex_records.len(), 1,
                    "Two events in the same hour bucket should produce 1 aggregated record, got {}",
                    codex_records.len()
                );

                // The summed total_tokens should equal sum of both events
                let expected_total = (tokens_a + tokens_a / 2 + tokens_b + tokens_b / 2) as i64;
                let actual_total = codex_records[0].total_tokens.unwrap_or(0);

                prop_assert_eq!(
                    actual_total, expected_total,
                    "Bucketed total ({}) should equal sum of both events ({})",
                    actual_total, expected_total
                );

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 5: No data loss in aggregation - The sum of tokens across all history
        /// records equals the sum of all individual events in that time range.
        #[test]
        fn prop_no_data_loss_in_aggregation(events in arb_today_events(8)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_no_loss.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                storage.store_events(&events).await.unwrap();

                // Query history over a range that covers all events (last 3 hours)
                let now = Utc::now();
                let time_range = TimeRange {
                    start: now - Duration::hours(3),
                    end: now + Duration::minutes(1),
                };
                let records = storage.get_history(&time_range, Granularity::Hourly).await.unwrap();

                // Sum total_tokens from all history records
                let history_total: i64 = records
                    .iter()
                    .filter_map(|r| r.total_tokens)
                    .sum();

                // Sum total_tokens from all input events
                let events_total: i64 = events
                    .iter()
                    .filter_map(|e| e.tokens.total_tokens.map(|t| t as i64))
                    .sum();

                prop_assert_eq!(
                    history_total, events_total,
                    "Sum of history records ({}) must equal sum of input events ({}). No data loss.",
                    history_total, events_total
                );

                // Also verify input tokens
                let history_input: i64 = records
                    .iter()
                    .filter_map(|r| r.input_tokens)
                    .sum();
                let events_input: i64 = events
                    .iter()
                    .filter_map(|e| e.tokens.input_tokens.map(|t| t as i64))
                    .sum();
                prop_assert_eq!(
                    history_input, events_input,
                    "Sum of history input_tokens ({}) must equal sum of input events ({})",
                    history_input, events_input
                );

                // And output tokens
                let history_output: i64 = records
                    .iter()
                    .filter_map(|r| r.output_tokens)
                    .sum();
                let events_output: i64 = events
                    .iter()
                    .filter_map(|e| e.tokens.output_tokens.map(|t| t as i64))
                    .sum();
                prop_assert_eq!(
                    history_output, events_output,
                    "Sum of history output_tokens ({}) must equal sum of input events ({})",
                    history_output, events_output
                );

                storage.close().await;
                Ok(())
            })?;
        }
    }
}


#[cfg(test)]
mod prop_tests_timestamp_consistency {
    use super::*;
    use crate::types::{EventFingerprint, EventType, QuotaUsage, ReconciledEvent, TokenUsage};
    use chrono::{DateTime, Utc};
    use proptest::prelude::*;
    use tempfile::TempDir;
    use uuid::Uuid;

    /// **Validates: Requirements 4.2, 9.4**

    // ─── Strategies ─────────────────────────────────────────────────────────────

    fn arb_event_type() -> impl Strategy<Value = EventType> {
        prop_oneof![
            Just(EventType::TokenCount),
            Just(EventType::SessionSummary),
            Just(EventType::QuotaSample),
            Just(EventType::DailyAggregate),
        ]
    }

    fn arb_provider_id() -> impl Strategy<Value = String> {
        prop_oneof![Just("codex".to_string()), Just("claude".to_string()),]
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
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0.0f64..200.0),
            proptest::option::of(0u64..10_000_000),
        )
            .prop_map(|(fast, standard, excess, daily)| QuotaUsage {
                fast_hours_pct: fast,
                standard_pct: standard,
                excess_pct: excess,
                daily_tokens: daily,
            })
    }

    /// Generate timestamps with sub-second (nanosecond) precision across a wide range.
    fn arb_timestamp_with_subsecond() -> impl Strategy<Value = DateTime<Utc>> {
        let min_ts = chrono::NaiveDate::from_ymd_opt(2023, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let max_ts = chrono::NaiveDate::from_ymd_opt(2025, 12, 31)
            .unwrap()
            .and_hms_opt(23, 59, 59)
            .unwrap()
            .and_utc()
            .timestamp();

        // Generate seconds + nanoseconds for sub-second precision
        (min_ts..max_ts, 0u32..999_999_999u32).prop_map(|(secs, nanos)| {
            DateTime::from_timestamp(secs, nanos).unwrap()
        })
    }

    fn arb_fingerprint() -> impl Strategy<Value = EventFingerprint> {
        proptest::collection::vec(any::<u8>(), 32).prop_map(|bytes| {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            EventFingerprint::new(arr)
        })
    }

    fn arb_reconciled_event_with_timestamp(
        ts: DateTime<Utc>,
    ) -> impl Strategy<Value = ReconciledEvent> {
        (
            arb_fingerprint(),
            arb_provider_id(),
            arb_event_type(),
            proptest::option::of("[a-z0-9\\-]{3,20}"),
            arb_token_usage(),
            proptest::option::of(1024u64..200_000),
            proptest::option::of(arb_quota_usage()),
            proptest::option::of("[a-f0-9]{16,32}"),
            proptest::option::of("[a-f0-9]{16,32}"),
            (1u8..5),
            arb_timestamp_with_subsecond(),
        )
            .prop_map(
                move |(
                    fingerprint,
                    provider_id,
                    event_type,
                    model,
                    tokens,
                    context_window,
                    quota,
                    session_hash,
                    project_hash,
                    source_count,
                    reconciled_at,
                )| {
                    ReconciledEvent {
                        id: Uuid::new_v4(),
                        fingerprint,
                        provider_id,
                        event_type,
                        timestamp: ts,
                        model,
                        tokens,
                        context_window,
                        quota,
                        session_hash,
                        project_hash,
                        reconciled_at,
                        source_count,
                    }
                },
            )
    }

    fn arb_reconciled_event() -> impl Strategy<Value = ReconciledEvent> {
        arb_timestamp_with_subsecond().prop_flat_map(|ts| arb_reconciled_event_with_timestamp(ts))
    }

    fn arb_events_vec(min: usize, max: usize) -> impl Strategy<Value = Vec<ReconciledEvent>> {
        proptest::collection::vec(arb_reconciled_event(), min..=max)
    }

    // ─── Property Tests ─────────────────────────────────────────────────────────

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(10))]

        /// Property 1: UTC preservation - timestamps stored and retrieved remain in UTC
        /// and are identical to the input within rfc3339 formatting precision.
        #[test]
        fn prop_utc_preservation(events in arb_events_vec(1, 10)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_ts.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                storage.store_events(&events).await.unwrap();

                // Retrieve each event's timestamp_utc and verify it matches the original
                for event in &events {
                    let row = sqlx::query(
                        "SELECT timestamp_utc FROM usage_events WHERE id = ?"
                    )
                    .bind(event.id.to_string())
                    .fetch_one(storage.pool())
                    .await
                    .unwrap();

                    let stored_ts_str: String = row.get("timestamp_utc");
                    let parsed_ts = DateTime::parse_from_rfc3339(&stored_ts_str)
                        .expect("stored timestamp must parse as RFC3339");

                    // The stored timestamp must be in UTC (offset +00:00)
                    prop_assert_eq!(
                        parsed_ts.offset().local_minus_utc(), 0,
                        "Stored timestamp must be in UTC, got offset: {}",
                        parsed_ts.offset()
                    );

                    // After rfc3339 round-trip, the timestamp should match the original
                    // (within rfc3339 precision - nanoseconds may be truncated)
                    let expected_rfc3339 = event.timestamp.to_rfc3339();
                    prop_assert_eq!(
                        &stored_ts_str, &expected_rfc3339,
                        "Stored timestamp '{}' must equal input rfc3339 '{}'",
                        stored_ts_str, expected_rfc3339
                    );
                }

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 2: ISO 8601 format - the stored timestamp_utc column contains a valid
        /// ISO 8601 string parseable by chrono.
        #[test]
        fn prop_iso8601_format(events in arb_events_vec(1, 10)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_iso.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                storage.store_events(&events).await.unwrap();

                // Query all stored timestamps
                let rows = sqlx::query("SELECT timestamp_utc FROM usage_events")
                    .fetch_all(storage.pool())
                    .await
                    .unwrap();

                for row in &rows {
                    let ts_str: String = row.get("timestamp_utc");

                    // Must parse as RFC3339 (which is a profile of ISO 8601)
                    let parsed = DateTime::parse_from_rfc3339(&ts_str);
                    prop_assert!(
                        parsed.is_ok(),
                        "Stored timestamp '{}' must be valid ISO 8601/RFC3339, parse error: {:?}",
                        ts_str,
                        parsed.err()
                    );

                    // Verify the string contains expected ISO 8601 markers
                    prop_assert!(
                        ts_str.contains('T'),
                        "ISO 8601 timestamp must contain 'T' separator, got: '{}'",
                        ts_str
                    );
                    // Must end with timezone indicator (e.g., +00:00 or Z)
                    prop_assert!(
                        ts_str.ends_with("+00:00") || ts_str.ends_with('Z'),
                        "ISO 8601 UTC timestamp must end with +00:00 or Z, got: '{}'",
                        ts_str
                    );
                }

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 3: Ordering preserved - events stored with ordered timestamps maintain
        /// their ordering when queried with ORDER BY timestamp_utc.
        #[test]
        fn prop_ordering_preserved(events in arb_events_vec(2, 15)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_order.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                storage.store_events(&events).await.unwrap();

                // Query events ordered by timestamp_utc
                let rows = sqlx::query(
                    "SELECT timestamp_utc FROM usage_events ORDER BY timestamp_utc ASC"
                )
                .fetch_all(storage.pool())
                .await
                .unwrap();

                // Verify the returned timestamps are in non-decreasing order
                let timestamps: Vec<String> = rows.iter()
                    .map(|r| r.get::<String, _>("timestamp_utc"))
                    .collect();

                for i in 1..timestamps.len() {
                    let prev = DateTime::parse_from_rfc3339(&timestamps[i - 1]).unwrap();
                    let curr = DateTime::parse_from_rfc3339(&timestamps[i]).unwrap();
                    prop_assert!(
                        prev <= curr,
                        "Timestamps must be in non-decreasing order after ORDER BY, \
                         but found '{}' > '{}' at positions {}/{}",
                        timestamps[i - 1], timestamps[i], i - 1, i
                    );
                }

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 4: No timezone drift - storing an event at a specific UTC time and
        /// retrieving it yields the same UTC time (no accidental timezone offset).
        #[test]
        fn prop_no_timezone_drift(events in arb_events_vec(1, 10)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_drift.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                storage.store_events(&events).await.unwrap();

                for event in &events {
                    let row = sqlx::query(
                        "SELECT timestamp_utc FROM usage_events WHERE id = ?"
                    )
                    .bind(event.id.to_string())
                    .fetch_one(storage.pool())
                    .await
                    .unwrap();

                    let stored_ts_str: String = row.get("timestamp_utc");
                    let retrieved_ts = DateTime::parse_from_rfc3339(&stored_ts_str)
                        .unwrap()
                        .with_timezone(&Utc);

                    // The original timestamp's rfc3339 representation should round-trip exactly
                    let original_rfc3339 = event.timestamp.to_rfc3339();
                    let original_reparsed = DateTime::parse_from_rfc3339(&original_rfc3339)
                        .unwrap()
                        .with_timezone(&Utc);

                    // No drift: retrieved timestamp must equal the original (within rfc3339 precision)
                    prop_assert_eq!(
                        retrieved_ts, original_reparsed,
                        "Timezone drift detected: stored={} retrieved={} original={}",
                        stored_ts_str, retrieved_ts, event.timestamp
                    );

                    // Extra check: the difference in seconds must be zero
                    let diff_secs = (retrieved_ts - original_reparsed).num_seconds().abs();
                    prop_assert_eq!(
                        diff_secs, 0,
                        "Timestamp drift of {} seconds detected for event {}",
                        diff_secs, event.id
                    );
                }

                storage.close().await;
                Ok(())
            })?;
        }

        /// Property 5: Sub-second precision - timestamps with sub-second precision
        /// survive the store/retrieve cycle within rfc3339 formatting precision.
        #[test]
        fn prop_subsecond_precision(events in arb_events_vec(1, 10)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let tmp_dir = TempDir::new().unwrap();
                let db_path = tmp_dir.path().join("test_subsec.db");
                let storage = StorageLayer::new(&db_path).await.unwrap();

                storage.store_events(&events).await.unwrap();

                for event in &events {
                    let row = sqlx::query(
                        "SELECT timestamp_utc FROM usage_events WHERE id = ?"
                    )
                    .bind(event.id.to_string())
                    .fetch_one(storage.pool())
                    .await
                    .unwrap();

                    let stored_ts_str: String = row.get("timestamp_utc");
                    let retrieved_ts = DateTime::parse_from_rfc3339(&stored_ts_str)
                        .unwrap()
                        .with_timezone(&Utc);

                    // The rfc3339 format preserves sub-second precision.
                    // Verify that chrono's to_rfc3339() output matches what was stored.
                    let expected_str = event.timestamp.to_rfc3339();
                    let expected_ts = DateTime::parse_from_rfc3339(&expected_str)
                        .unwrap()
                        .with_timezone(&Utc);

                    // The retrieved timestamp must match within rfc3339 precision
                    prop_assert_eq!(
                        retrieved_ts, expected_ts,
                        "Sub-second precision lost: original={} (ns={}), stored='{}', retrieved={}",
                        event.timestamp,
                        event.timestamp.timestamp_subsec_nanos(),
                        stored_ts_str,
                        retrieved_ts
                    );

                    // If original had sub-second component, verify it's preserved
                    if event.timestamp.timestamp_subsec_nanos() > 0 {
                        // The stored string should contain a fractional seconds part
                        // (chrono's to_rfc3339 includes fractional seconds when non-zero)
                        let has_fractional = stored_ts_str.contains('.');
                        prop_assert!(
                            has_fractional,
                            "Timestamp with sub-second precision ({}ns) should have \
                             fractional part in stored string, got: '{}'",
                            event.timestamp.timestamp_subsec_nanos(),
                            stored_ts_str
                        );
                    }
                }

                storage.close().await;
                Ok(())
            })?;
        }
    }
}
