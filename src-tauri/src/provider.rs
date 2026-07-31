use chrono::{DateTime, Utc};

use crate::error::CollectionError;
use crate::types::{QuotaUsage, RawUsageEvent};

/// Metadata about the collection process from a single provider invocation.
#[derive(Debug, Clone)]
pub struct SourceMetadata {
    /// Number of files read during this collection.
    pub files_read: u32,
    /// Total bytes processed across all files.
    pub bytes_processed: u64,
    /// Non-fatal errors encountered during collection.
    pub errors: Vec<String>,
}

/// Result returned by a successful provider collection.
#[derive(Debug, Clone)]
pub struct CollectionResult {
    /// Raw usage events extracted from the provider's data sources.
    pub events: Vec<RawUsageEvent>,
    /// Checkpoint timestamp marking the latest data processed.
    pub checkpoint: DateTime<Utc>,
    /// Metadata about the collection operation.
    pub source_metadata: SourceMetadata,
}

/// Summary of a provider's current state for display in the widget.
#[derive(Debug, Clone)]
pub struct ProviderSummary {
    /// Unique identifier for this provider (e.g., "codex", "claude").
    pub provider_id: String,
    /// Human-readable name (e.g., "Codex Desktop", "Claude Desktop").
    pub display_name: String,
    /// Whether the provider's data sources are currently accessible.
    pub is_available: bool,
    /// The current/latest model in use, if known.
    pub current_model: Option<String>,
    /// Total tokens consumed today, if available.
    pub tokens_today: Option<u64>,
    /// Current quota usage, if applicable.
    pub quota: Option<QuotaUsage>,
    /// Context window size of the current model, if known.
    pub context_window: Option<u64>,
    /// Timestamp of the most recent activity from this provider.
    pub last_activity: Option<DateTime<Utc>>,
    /// When the quota window resets, for providers that publish one.
    pub quota_resets_at: Option<DateTime<Utc>>,
}

/// Trait defining the interface for data collection adapters.
///
/// Each AI application (Codex Desktop, Claude Desktop, etc.) implements
/// this trait to provide a uniform collection interface. Implementations
/// must be Send + Sync for use across async task boundaries.
pub trait ProviderAdapter: Send + Sync {
    /// Returns the unique identifier for this provider (e.g., "codex").
    fn provider_id(&self) -> &str;

    /// Returns the human-readable display name (e.g., "Codex Desktop").
    fn display_name(&self) -> &str;

    /// Checks whether the provider's data sources are currently accessible.
    fn is_available(&self) -> bool;

    /// Collects usage events from the provider's data sources.
    ///
    /// If `since` is provided, only events newer than that timestamp are collected.
    /// Otherwise, all available data is collected.
    fn collect(&self, since: Option<DateTime<Utc>>) -> Result<CollectionResult, CollectionError>;

    /// Returns a summary of the provider's current state for widget display.
    fn get_current_summary(&self) -> Result<ProviderSummary, CollectionError>;

    /// Returns the timestamp of the last successful collection checkpoint.
    fn last_checkpoint(&self) -> Option<DateTime<Utc>>;

    /// When the provider's quota window resets, if it publishes one.
    fn quota_resets_at(&self) -> Option<DateTime<Utc>> {
        None
    }

    /// Export the adapter's incremental-collection state so it can be persisted.
    ///
    /// Adapters that read nothing incrementally keep the default (empty state).
    fn export_state(&self) -> ProviderState {
        ProviderState::default()
    }

    /// Restore previously persisted incremental-collection state.
    ///
    /// Called once at startup, before the first collection cycle, so that a
    /// restart does not re-read every source file from byte 0.
    fn restore_state(&self, _state: &ProviderState) {}
}

/// Incremental-collection state of a provider, persisted between runs.
#[derive(Debug, Clone, Default)]
pub struct ProviderState {
    /// Byte offset already consumed, per source file path.
    pub file_positions: std::collections::HashMap<String, u64>,
    /// Timestamp of the newest event seen so far.
    pub checkpoint: Option<DateTime<Utc>>,
    /// Adapter-specific JSON blob (e.g. the last known quota snapshot), kept so
    /// values that only appear while parsing survive a restart.
    pub metadata: Option<String>,
}
