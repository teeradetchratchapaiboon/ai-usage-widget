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
        }
    }
}

impl AppConfig {
    /// Load configuration from a JSON file, falling back to defaults for missing fields.
    pub fn load_from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)
            .map_err(|_| ConfigError::FileNotFound(path.display().to_string()))?;

        serde_json::from_str(&content)
            .map_err(|e| ConfigError::InvalidFormat(e.to_string()))
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

/// Widget window configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    /// Window width in logical pixels (default: 340).
    pub width: u32,

    /// Window height in logical pixels (default: 200).
    pub height: u32,

    /// Whether the widget is always on top (default: true).
    pub always_on_top: bool,

    /// Whether click-through mode is enabled (default: false).
    pub click_through: bool,

    /// Persisted X position (logical pixels).
    pub position_x: Option<i32>,

    /// Persisted Y position (logical pixels).
    pub position_y: Option<i32>,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 340,
            height: 200,
            always_on_top: true,
            click_through: false,
            position_x: None,
            position_y: None,
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
