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

/// When each quota window rolls over, for providers that publish it.
///
/// Both vendors meter two windows independently — a short rolling one and a
/// long one — and a single "resets at" cannot describe them: hitting the
/// weekly cap says nothing about when the next five hours open up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuotaResets {
    /// Short rolling window: Codex's 300-minute limit, Claude's `fh`.
    pub fast_hours: Option<DateTime<Utc>>,
    /// Long window: Codex's 10080-minute limit, Claude's `sd`.
    pub weekly: Option<DateTime<Utc>>,
    /// True when these times were derived rather than published.
    ///
    /// Codex states its reset timestamps outright; Claude states nothing and
    /// ours are reconstructed from where its usage history dropped. Both end
    /// up in the same field, so without this flag the UI would present a
    /// reconstruction with the same confidence as a fact.
    pub estimated: bool,
}

impl QuotaResets {
    /// True when neither window published a reset time.
    pub fn is_empty(&self) -> bool {
        self.fast_hours.is_none() && self.weekly.is_none()
    }
}

/// When each quota window's percentage was actually written by its source.
///
/// Kept separate from [`QuotaResets`] because the two answer different
/// questions: a reset time says when the number will change, this says how much
/// the number can still be trusted. A percentage is only ever as current as the
/// record it came from — the collection cycle that noticed it says nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuotaObservations {
    /// Source timestamp of the short-window percentage.
    pub fast_hours: Option<DateTime<Utc>>,
    /// Source timestamp of the long-window percentage.
    pub weekly: Option<DateTime<Utc>>,
}

impl QuotaObservations {
    /// True when neither window carries a usable observation time.
    pub fn is_empty(&self) -> bool {
        self.fast_hours.is_none() && self.weekly.is_none()
    }
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
    /// When each quota window resets, for providers that publish it.
    pub quota_resets: QuotaResets,
    /// When each quota window's percentage was written by its source.
    pub quota_observed: QuotaObservations,
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

    /// When the provider's quota windows reset, if it publishes them.
    fn quota_resets(&self) -> QuotaResets {
        QuotaResets::default()
    }

    /// When each quota percentage was written by the source that supplied it.
    ///
    /// Adapters that cannot tell return the default (unknown), which the UI
    /// renders as an unknown age — never as a current reading.
    fn quota_observed(&self) -> QuotaObservations {
        QuotaObservations::default()
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
