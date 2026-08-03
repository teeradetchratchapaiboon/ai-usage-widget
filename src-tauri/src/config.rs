use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::ConfigError;

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Path to the SQLite database file.
    pub db_path: PathBuf,

    /// Codex Desktop provider settings.
    pub codex: CodexConfig,

    /// Claude Desktop provider settings.
    pub claude: ClaudeConfig,

    /// Widget window settings.
    pub window: WindowConfig,

    /// Network allowlist/blocklist settings.
    pub network: NetworkConfig,

    /// Data collection interval in seconds (default: 30).
    pub collection_interval_secs: u32,

    /// Data retention period in days (default: 365).
    pub retention_days: u32,

    /// UI locale ("th" or "en").
    pub locale: String,

    /// Quota percentage that triggers a warning notification (default: 75).
    #[serde(default = "default_warning_pct")]
    pub notification_warning_pct: f64,

    /// Quota percentage that triggers a critical notification (default: 90).
    #[serde(default = "default_critical_pct")]
    pub notification_critical_pct: f64,
}

/// Default warning threshold for quota notifications.
fn default_warning_pct() -> f64 {
    75.0
}

/// Default critical threshold for quota notifications.
fn default_critical_pct() -> f64 {
    90.0
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            db_path: default_db_path(),
            codex: CodexConfig::default(),
            claude: ClaudeConfig::default(),
            window: WindowConfig::default(),
            network: NetworkConfig::default(),
            collection_interval_secs: 30,
            retention_days: 365,
            locale: "th".to_string(),
            notification_warning_pct: default_warning_pct(),
            notification_critical_pct: default_critical_pct(),
        }
    }
}

impl AppConfig {
    /// Load configuration from a JSON file, falling back to defaults for missing fields.
    pub fn load_from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)
            .map_err(|_| ConfigError::FileNotFound(path.display().to_string()))?;

        let mut config: Self = serde_json::from_str(&content)
            .map_err(|e| ConfigError::InvalidFormat(e.to_string()))?;

        config.window.normalize();

        Ok(config)
    }

    /// Load configuration from a JSON file, using defaults if the file doesn't exist.
    pub fn load_or_default(path: &Path) -> Self {
        Self::load_from_file(path).unwrap_or_default()
    }

    /// Save the current configuration to a JSON file.
    pub fn save_to_file(&self, path: &Path) -> Result<(), ConfigError> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| ConfigError::InvalidFormat(e.to_string()))?;

        std::fs::write(path, content)
            .map_err(|e| ConfigError::InvalidFormat(format!("failed to write config: {}", e)))
    }
}

/// Configuration for the Codex Desktop provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexConfig {
    /// Path to Codex sessions directory (default: ~/.codex/sessions/).
    pub sessions_dir: PathBuf,

    /// Path to Codex state database (default: ~/.codex/state_5.sqlite).
    pub state_db_path: PathBuf,

    /// Whether Codex data collection is enabled.
    pub enabled: bool,
}

impl Default for CodexConfig {
    fn default() -> Self {
        let home = dirs_fallback_home();
        Self {
            sessions_dir: home.join(".codex").join("sessions"),
            state_db_path: home.join(".codex").join("state_5.sqlite"),
            enabled: true,
        }
    }
}

/// Configuration for the Claude Desktop provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeConfig {
    /// Path to Claude Desktop data directory.
    pub data_dir: PathBuf,

    /// Whether Claude data collection is enabled.
    pub enabled: bool,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            data_dir: default_claude_data_dir(),
            enabled: true,
        }
    }
}

/// Width a fresh widget opens at, in logical pixels.
pub const DEFAULT_WIDGET_WIDTH: u32 = 360;

/// Height a fresh widget opens at, in logical pixels.
///
/// Measured rather than guessed. Header, footer and padding take 73px; a
/// provider whose two windows each carry a freshness caption *and* a reset
/// countdown takes about 111px, so two of them need ~222px and an excess row
/// adds ~25px more. 340 leaves the rows 267px — enough for the worst case
/// without the widget growing into screen clutter, which is the other half of
/// the complaint.
pub const DEFAULT_WIDGET_HEIGHT: u32 = 340;

/// Heights shipped by earlier builds, in order.
///
/// A config still carrying one of these was never resized by hand, so it
/// adopts the current default rather than keeping a size that now clips.
pub const LEGACY_WIDGET_HEIGHTS: [u32; 2] = [200, 300];

/// Widest a remembered widget may be before it is treated as damage.
///
/// The widget used to offer a maximize button. Maximizing fired a resize like
/// any other, so the screen width was persisted as the widget's own — and a
/// later restore could pair that width with the normal height, leaving a
/// letterbox pinned across the display. The button is gone; this repairs the
/// configs it already wrote.
pub const MAX_SENSIBLE_WIDGET_WIDTH: u32 = 900;

/// Tallest a remembered widget may be, for the same reason.
pub const MAX_SENSIBLE_WIDGET_HEIGHT: u32 = 900;

/// Widget window configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    /// Window width in logical pixels (default: 340).
    pub width: u32,

    /// Window height in logical pixels (default: 300).
    pub height: u32,

    /// Whether the widget is always on top (default: true).
    pub always_on_top: bool,

    /// Whether click-through mode is enabled (default: false).
    pub click_through: bool,

    /// Persisted X position (logical pixels).
    pub position_x: Option<i32>,

    /// Persisted Y position (logical pixels).
    pub position_y: Option<i32>,

    /// Whether the widget was left collapsed to its header strip.
    ///
    /// Defaulted so a config written before this field still loads.
    #[serde(default)]
    pub collapsed: bool,
}

impl WindowConfig {
    /// Bring a loaded size back into the range the widget can actually use.
    ///
    /// Two separate repairs, both for sizes written by earlier builds rather
    /// than chosen by the user:
    ///
    /// - a height still equal to an old default is adopted forward, since it
    ///   was never customised and now clips the second provider
    /// - a size larger than any sensible widget is discarded, because the only
    ///   way to get one was the maximize button that no longer exists
    pub fn normalize(&mut self) {
        if LEGACY_WIDGET_HEIGHTS.contains(&self.height) {
            self.height = DEFAULT_WIDGET_HEIGHT;
        }
        if self.width > MAX_SENSIBLE_WIDGET_WIDTH {
            self.width = DEFAULT_WIDGET_WIDTH;
        }
        if self.height > MAX_SENSIBLE_WIDGET_HEIGHT {
            self.height = DEFAULT_WIDGET_HEIGHT;
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDGET_WIDTH,
            height: DEFAULT_WIDGET_HEIGHT,
            always_on_top: true,
            click_through: false,
            position_x: None,
            position_y: None,
            collapsed: false,
        }
    }
}

/// Network security configuration for the zero-token guarantee.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Hosts that are allowed for outbound requests (e.g., api.github.com for updates).
    pub allowed_hosts: Vec<String>,

    /// URL patterns that are blocked (model inference endpoints).
    pub blocked_patterns: Vec<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            allowed_hosts: vec!["api.github.com".to_string()],
            blocked_patterns: vec![
                "api.openai.com".to_string(),
                "api.anthropic.com".to_string(),
            ],
        }
    }
}

/// Resolve the default database path based on whether we're in portable or installed mode.
fn default_db_path() -> PathBuf {
    // Check for portable mode (portable.flag next to executable)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            if exe_dir.join("portable.flag").exists() {
                return exe_dir.join("data").join("usage.db");
            }
        }
    }

    // Installed mode: use %APPDATA%/ai-usage-widget/
    if let Some(app_data) = std::env::var_os("APPDATA") {
        return PathBuf::from(app_data)
            .join("ai-usage-widget")
            .join("usage.db");
    }

    // Fallback
    PathBuf::from("usage.db")
}

/// Resolve the default Claude Desktop data directory (Windows Store sandboxed path).
fn default_claude_data_dir() -> PathBuf {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("Packages")
            .join("Claude_pzs8sxrjxfjjc")
            .join("LocalCache")
            .join("Roaming")
            .join("Claude");
    }

    // Fallback
    PathBuf::from("claude-data")
}

/// Get the user home directory with a simple fallback.
fn dirs_fallback_home() -> PathBuf {
    if let Some(home) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(home);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home);
    }
    PathBuf::from(".")
}

#[cfg(test)]
mod window_size_normalisation_tests {
    use super::*;

    fn cfg(width: u32, height: u32) -> WindowConfig {
        WindowConfig {
            width,
            height,
            ..Default::default()
        }
    }

    #[test]
    fn test_an_old_default_height_is_adopted_forward() {
        // Never customised, and now too short for the per-window quota rows
        for legacy in LEGACY_WIDGET_HEIGHTS {
            let mut c = cfg(DEFAULT_WIDGET_WIDTH, legacy);
            c.normalize();
            assert_eq!(c.height, DEFAULT_WIDGET_HEIGHT, "legacy height {legacy}");
        }
    }

    #[test]
    fn test_a_screen_sized_config_is_repaired() {
        // Written by the maximize button that no longer exists
        let mut c = cfg(2560, 1400);
        c.normalize();

        assert_eq!(c.width, DEFAULT_WIDGET_WIDTH);
        assert_eq!(c.height, DEFAULT_WIDGET_HEIGHT);
    }

    #[test]
    fn test_a_wide_but_sane_size_is_left_alone() {
        // Someone who dragged the widget wider on purpose keeps their size
        let mut c = cfg(640, 560);
        c.normalize();

        assert_eq!((c.width, c.height), (640, 560));
    }

    #[test]
    fn test_a_stretched_width_alone_is_repaired() {
        // The reported shape: screen-wide but normal height
        let mut c = cfg(2560, DEFAULT_WIDGET_HEIGHT);
        c.normalize();

        assert_eq!(c.width, DEFAULT_WIDGET_WIDTH);
        assert_eq!(c.height, DEFAULT_WIDGET_HEIGHT);
    }
}
