use thiserror::Error;

/// Errors that can occur during data collection from providers.
#[derive(Debug, Error)]
pub enum CollectionError {
    #[error("data source unavailable: {0} - {1}")]
    DataSourceUnavailable(String, String),

    #[error("parse error: {0}")]
    ParseError(#[from] ParseError),

    #[error("database locked: {0}")]
    DatabaseLocked(String),

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("timeout after {0}ms: {1}")]
    Timeout(u64, String),
}

/// Errors that can occur in the storage layer.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("connection failed: {0}")]
    ConnectionFailed(String),

    #[error("query failed: {0}")]
    QueryFailed(String),

    #[error("migration failed: {0}")]
    MigrationFailed(String),

    #[error("backup failed: {0}")]
    BackupFailed(String),

    #[error("restore failed: {0}")]
    RestoreFailed(String),

    #[error("checksum mismatch: expected {0}, got {1}")]
    ChecksumMismatch(String, String),
}

/// Errors related to input validation (IPC commands).
#[derive(Debug, Error, PartialEq)]
pub enum ValidationError {
    #[error("invalid range: {0}")]
    InvalidRange(String),

    #[error("range too large: {0} days exceeds maximum of {1} days")]
    RangeTooLarge(u32, u32),

    #[error("future date not allowed: {0}")]
    FutureDate(String),

    #[error("invalid locale: {0}")]
    InvalidLocale(String),

    #[error("invalid interval: {0}s (must be between {1}s and {2}s)")]
    InvalidInterval(u32, u32, u32),

    #[error("invalid retention days: {0} (must be between {1} and {2})")]
    InvalidRetentionDays(u32, u32, u32),

    #[error("invalid provider id: {0}")]
    InvalidProviderId(String),

    #[error("invalid notification threshold: {0}")]
    InvalidThreshold(String),
}

/// Errors that occur while parsing provider data files.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("malformed JSON: {0}")]
    MalformedJson(String),

    #[error("unexpected schema: {0}")]
    UnexpectedSchema(String),

    #[error("missing field: {0}")]
    MissingField(String),

    #[error("invalid timestamp: {0} - {1}")]
    InvalidTimestamp(String, String),
}

/// Errors related to application configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config file not found: {0}")]
    FileNotFound(String),

    #[error("invalid config format: {0}")]
    InvalidFormat(String),

    #[error("missing required config field: {0}")]
    MissingRequired(String),
}
