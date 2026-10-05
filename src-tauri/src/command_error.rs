//! Structured errors returned by IPC commands.
//!
//! A command that fails with a [`CommandError`] serializes to
//! `{"code":"INVALID_INTERVAL","message":"...","params":{...}}`. The frontend
//! translates `code` (with `params` as interpolation values) through the
//! `errors.code.*` i18n keys and falls back to `message`, which always keeps
//! the full English detail for logs.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use crate::error::{RestoreRejection, StorageError, ValidationError};

/// Stable error codes. Every variant needs an `errors.code.<CODE>` entry in
/// both `src/i18n/en.json` and `src/i18n/th.json` (enforced by a test below).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidLocale,
    InvalidInterval,
    InvalidRetention,
    ThresholdOutOfRange,
    ThresholdOrder,
    InvalidTimeRange,
    TimeRangeTooLarge,
    FutureDate,
    InvalidProviderId,
    SettingsSaveFailed,
    AutostartFailed,
    WindowSettingFailed,
    BackupPathEmpty,
    BackupFailed,
    RestorePathEmpty,
    RestoreFileNotFound,
    RestoreFromLiveDatabase,
    RestoreInvalidBackup,
    RestoreIntegrityFailed,
    RestoreChecksumMismatch,
    RestoreFailed,
    RestoreReloadFailed,
    UpdateBlocked,
    UpdateCheckFailed,
    UpdateNotAvailable,
    UpdateInstallFailed,
    Internal,
}

impl ErrorCode {
    /// Every code, for the i18n catalog check.
    pub const ALL: &'static [ErrorCode] = &[
        ErrorCode::InvalidLocale,
        ErrorCode::InvalidInterval,
        ErrorCode::InvalidRetention,
        ErrorCode::ThresholdOutOfRange,
        ErrorCode::ThresholdOrder,
        ErrorCode::InvalidTimeRange,
        ErrorCode::TimeRangeTooLarge,
        ErrorCode::FutureDate,
        ErrorCode::InvalidProviderId,
        ErrorCode::SettingsSaveFailed,
        ErrorCode::AutostartFailed,
        ErrorCode::WindowSettingFailed,
        ErrorCode::BackupPathEmpty,
        ErrorCode::BackupFailed,
        ErrorCode::RestorePathEmpty,
        ErrorCode::RestoreFileNotFound,
        ErrorCode::RestoreFromLiveDatabase,
        ErrorCode::RestoreInvalidBackup,
        ErrorCode::RestoreIntegrityFailed,
        ErrorCode::RestoreChecksumMismatch,
        ErrorCode::RestoreFailed,
        ErrorCode::RestoreReloadFailed,
        ErrorCode::UpdateBlocked,
        ErrorCode::UpdateCheckFailed,
        ErrorCode::UpdateNotAvailable,
        ErrorCode::UpdateInstallFailed,
        ErrorCode::Internal,
    ];
}

/// Error payload of the converted IPC commands.
#[derive(Debug, Serialize)]
pub struct CommandError {
    pub code: ErrorCode,
    /// Full English detail; shown when the frontend has no translation.
    pub message: String,
    /// Interpolation values for the translated text.
    pub params: BTreeMap<String, String>,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            params: BTreeMap::new(),
        }
    }

    pub fn param(mut self, key: &str, value: impl ToString) -> Self {
        self.params.insert(key.to_string(), value.to_string());
        self
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<ValidationError> for CommandError {
    fn from(e: ValidationError) -> Self {
        let message = format!("Settings validation failed: {}", e);
        match e {
            ValidationError::InvalidLocale(value) => {
                CommandError::new(ErrorCode::InvalidLocale, message).param("value", value)
            }
            ValidationError::InvalidInterval(value, min, max) => {
                CommandError::new(ErrorCode::InvalidInterval, message)
                    .param("value", value)
                    .param("min", min)
                    .param("max", max)
            }
            ValidationError::InvalidRetentionDays(value, min, max) => {
                CommandError::new(ErrorCode::InvalidRetention, message)
                    .param("value", value)
                    .param("min", min)
                    .param("max", max)
            }
            ValidationError::ThresholdOutOfRange { value, .. } => {
                CommandError::new(ErrorCode::ThresholdOutOfRange, message).param("value", value)
            }
            ValidationError::ThresholdOrder { warning, critical } => {
                CommandError::new(ErrorCode::ThresholdOrder, message)
                    .param("warning", warning)
                    .param("critical", critical)
            }
            ValidationError::InvalidRange(_) => {
                CommandError::new(ErrorCode::InvalidTimeRange, message)
            }
            ValidationError::RangeTooLarge(days, max) => {
                CommandError::new(ErrorCode::TimeRangeTooLarge, message)
                    .param("days", days)
                    .param("max", max)
            }
            ValidationError::FutureDate(_) => CommandError::new(ErrorCode::FutureDate, message),
            ValidationError::InvalidProviderId(_) => {
                CommandError::new(ErrorCode::InvalidProviderId, message)
            }
        }
    }
}

/// Map a failed `Storage::restore` to its error code.
pub fn restore_error(e: StorageError) -> CommandError {
    let code = match &e {
        StorageError::RestoreRejected(kind, _) => match kind {
            RestoreRejection::SourceMissing => ErrorCode::RestoreFileNotFound,
            RestoreRejection::LiveDatabase => ErrorCode::RestoreFromLiveDatabase,
            RestoreRejection::NotADatabase | RestoreRejection::MissingTable => {
                ErrorCode::RestoreInvalidBackup
            }
            RestoreRejection::IntegrityCheckFailed => ErrorCode::RestoreIntegrityFailed,
        },
        StorageError::ChecksumMismatch(..) => ErrorCode::RestoreChecksumMismatch,
        _ => ErrorCode::RestoreFailed,
    };
    CommandError::new(code, format!("Restore failed: {}", e))
}

/// Map a failed `Storage::backup` to its error code.
pub fn backup_error(e: StorageError) -> CommandError {
    CommandError::new(ErrorCode::BackupFailed, format!("Backup failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn validation_errors_map_to_codes_and_params() {
        let cases = [
            (
                ValidationError::InvalidLocale("fr".into()),
                ErrorCode::InvalidLocale,
                params(&[("value", "fr")]),
            ),
            (
                ValidationError::InvalidInterval(5, 10, 3600),
                ErrorCode::InvalidInterval,
                params(&[("value", "5"), ("min", "10"), ("max", "3600")]),
            ),
            (
                ValidationError::InvalidRetentionDays(0, 1, 3650),
                ErrorCode::InvalidRetention,
                params(&[("value", "0"), ("min", "1"), ("max", "3650")]),
            ),
            (
                ValidationError::ThresholdOutOfRange {
                    label: "warning".into(),
                    value: 101.0,
                },
                ErrorCode::ThresholdOutOfRange,
                params(&[("value", "101")]),
            ),
            (
                ValidationError::ThresholdOrder {
                    warning: 95.0,
                    critical: 90.5,
                },
                ErrorCode::ThresholdOrder,
                params(&[("warning", "95"), ("critical", "90.5")]),
            ),
            (
                ValidationError::InvalidRange("start must be before end".into()),
                ErrorCode::InvalidTimeRange,
                params(&[]),
            ),
            (
                ValidationError::RangeTooLarge(400, 366),
                ErrorCode::TimeRangeTooLarge,
                params(&[("days", "400"), ("max", "366")]),
            ),
            (
                ValidationError::FutureDate("x".into()),
                ErrorCode::FutureDate,
                params(&[]),
            ),
            (
                ValidationError::InvalidProviderId("x".into()),
                ErrorCode::InvalidProviderId,
                params(&[]),
            ),
        ];
        for (err, code, expected) in cases {
            let text = format!("Settings validation failed: {}", err);
            let mapped = CommandError::from(err);
            assert_eq!(mapped.code, code);
            assert_eq!(mapped.params, expected, "{:?}", code);
            assert_eq!(mapped.message, text);
        }
    }

    #[test]
    fn threshold_messages_keep_the_original_text() {
        let order = CommandError::from(ValidationError::ThresholdOrder {
            warning: 95.0,
            critical: 90.0,
        });
        assert_eq!(
            order.message,
            "Settings validation failed: invalid notification threshold: warning threshold (95) must be below critical threshold (90)"
        );
        let range = CommandError::from(ValidationError::ThresholdOutOfRange {
            label: "critical".into(),
            value: -1.0,
        });
        assert_eq!(
            range.message,
            "Settings validation failed: invalid notification threshold: critical threshold must be between 0 and 100, got -1"
        );
    }

    #[test]
    fn serializes_to_code_message_params() {
        let value = serde_json::to_value(CommandError::from(ValidationError::InvalidInterval(
            5, 10, 3600,
        )))
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "code": "INVALID_INTERVAL",
                "message": "Settings validation failed: invalid interval: 5s (must be between 10s and 3600s)",
                "params": { "value": "5", "min": "10", "max": "3600" }
            })
        );
    }

    #[test]
    fn restore_errors_map_by_kind() {
        let cases = [
            (
                RestoreRejection::SourceMissing,
                ErrorCode::RestoreFileNotFound,
            ),
            (
                RestoreRejection::LiveDatabase,
                ErrorCode::RestoreFromLiveDatabase,
            ),
            (
                RestoreRejection::NotADatabase,
                ErrorCode::RestoreInvalidBackup,
            ),
            (
                RestoreRejection::MissingTable,
                ErrorCode::RestoreInvalidBackup,
            ),
            (
                RestoreRejection::IntegrityCheckFailed,
                ErrorCode::RestoreIntegrityFailed,
            ),
        ];
        for (kind, code) in cases {
            let mapped = restore_error(StorageError::RestoreRejected(kind, "detail".into()));
            assert_eq!(mapped.code, code, "{:?}", kind);
            assert_eq!(mapped.message, "Restore failed: restore failed: detail");
        }

        assert_eq!(
            restore_error(StorageError::ChecksumMismatch("a".into(), "b".into())).code,
            ErrorCode::RestoreChecksumMismatch
        );
        assert_eq!(
            restore_error(StorageError::RestoreFailed("x".into())).code,
            ErrorCode::RestoreFailed
        );
        assert_eq!(
            backup_error(StorageError::BackupFailed("x".into())).code,
            ErrorCode::BackupFailed
        );
    }

    fn code_name(code: ErrorCode) -> String {
        serde_json::to_value(code)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn i18n_catalogs_cover_every_code() {
        let names: Vec<String> = ErrorCode::ALL.iter().map(|c| code_name(*c)).collect();
        let expected: BTreeSet<String> = names.iter().cloned().collect();
        assert_eq!(expected.len(), names.len(), "duplicate in ErrorCode::ALL");

        for (lang, raw) in [
            ("en", include_str!("../../src/i18n/en.json")),
            ("th", include_str!("../../src/i18n/th.json")),
        ] {
            let json: serde_json::Value = serde_json::from_str(raw).unwrap();
            let keys: BTreeSet<String> = json["errors"]["code"]
                .as_object()
                .unwrap_or_else(|| panic!("{lang}.json has no errors.code"))
                .keys()
                .cloned()
                .collect();
            assert_eq!(keys, expected, "{lang}.json errors.code keys");
        }
    }

    #[test]
    fn all_lists_every_variant() {
        // Exhaustive match: adding a variant without listing it in ALL fails
        // to compile here, and the count check catches a missing entry.
        let count = ErrorCode::ALL
            .iter()
            .map(|c| match c {
                ErrorCode::InvalidLocale
                | ErrorCode::InvalidInterval
                | ErrorCode::InvalidRetention
                | ErrorCode::ThresholdOutOfRange
                | ErrorCode::ThresholdOrder
                | ErrorCode::InvalidTimeRange
                | ErrorCode::TimeRangeTooLarge
                | ErrorCode::FutureDate
                | ErrorCode::InvalidProviderId
                | ErrorCode::SettingsSaveFailed
                | ErrorCode::AutostartFailed
                | ErrorCode::WindowSettingFailed
                | ErrorCode::BackupPathEmpty
                | ErrorCode::BackupFailed
                | ErrorCode::RestorePathEmpty
                | ErrorCode::RestoreFileNotFound
                | ErrorCode::RestoreFromLiveDatabase
                | ErrorCode::RestoreInvalidBackup
                | ErrorCode::RestoreIntegrityFailed
                | ErrorCode::RestoreChecksumMismatch
                | ErrorCode::RestoreFailed
                | ErrorCode::RestoreReloadFailed
                | ErrorCode::UpdateBlocked
                | ErrorCode::UpdateCheckFailed
                | ErrorCode::UpdateNotAvailable
                | ErrorCode::UpdateInstallFailed
                | ErrorCode::Internal => 1,
            })
            .sum::<usize>();
        assert_eq!(count, 27);
    }
}
