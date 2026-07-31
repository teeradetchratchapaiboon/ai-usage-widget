use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::config::WindowConfig;

/// Persisted window position data for multi-monitor support.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowPosition {
    /// Logical X coordinate.
    pub x: i32,
    /// Logical Y coordinate.
    pub y: i32,
    /// Monitor identifier (index or name) where the window was last placed.
    pub monitor: Option<String>,
}

/// Manages the compact widget and dashboard windows.
///
/// Handles creation, toggling, always-on-top behavior, DPI-independent sizing
/// via LogicalSize, and multi-monitor position persistence.
pub struct WindowManager {
    /// Label for the compact widget window.
    compact_label: String,
    /// Label for the dashboard window.
    dashboard_label: String,
    /// Whether the dashboard is currently open.
    dashboard_open: Mutex<bool>,
    /// Current window configuration (dimensions, always-on-top, position).
    config: Mutex<WindowConfig>,
    /// Path to persist window position data.
    position_file: PathBuf,
    /// Whether click-through mode is currently active.
    click_through: AtomicBool,
    /// Whether the widget is hidden due to a fullscreen app.
    hidden_for_fullscreen: AtomicBool,
}

impl WindowManager {
    /// Create a new WindowManager with the given config and persistence path.
    pub fn new(config: WindowConfig, data_dir: &Path) -> Self {
        let click_through_initial = config.click_through;
        Self {
            compact_label: "main".to_string(),
            dashboard_label: "dashboard".to_string(),
            dashboard_open: Mutex::new(false),
            config: Mutex::new(config),
            position_file: data_dir.join("window_position.json"),
            click_through: AtomicBool::new(click_through_initial),
            hidden_for_fullscreen: AtomicBool::new(false),
        }
    }

    /// Get the compact widget window label.
    pub fn compact_label(&self) -> &str {
        &self.compact_label
    }

    /// Get the dashboard window label.
    pub fn dashboard_label(&self) -> &str {
        &self.dashboard_label
    }

    /// Returns the logical size for the compact widget (340x200).
    /// Uses LogicalSize for DPI-independent rendering.
    pub fn compact_widget_size(&self) -> (u32, u32) {
        let cfg = self.config.lock().unwrap();
        (cfg.width, cfg.height)
    }

    /// Check if the dashboard is currently open.
    pub fn is_dashboard_open(&self) -> bool {
        *self.dashboard_open.lock().unwrap()
    }

    /// Mark the dashboard as closed (called when dashboard window is destroyed).
    pub fn mark_dashboard_closed(&self) {
        *self.dashboard_open.lock().unwrap() = false;
    }

    /// Mark the dashboard as open.
    pub fn mark_dashboard_open(&self) {
        *self.dashboard_open.lock().unwrap() = true;
    }

    /// Get the current always-on-top state.
    pub fn is_always_on_top(&self) -> bool {
        self.config.lock().unwrap().always_on_top
    }

    /// Update the always-on-top setting in config.
    pub fn set_always_on_top_config(&self, enabled: bool) {
        self.config.lock().unwrap().always_on_top = enabled;
    }

    /// Get the persisted position, if any.
    pub fn get_persisted_position(&self) -> Option<WindowPosition> {
        let cfg = self.config.lock().unwrap();
        match (cfg.position_x, cfg.position_y) {
            (Some(x), Some(y)) => Some(WindowPosition {
                x,
                y,
                monitor: None,
            }),
            _ => None,
        }
    }

    /// Persist the window position to config and to disk.
    pub fn persist_position(&self, x: i32, y: i32, monitor: Option<String>) {
        {
            let mut cfg = self.config.lock().unwrap();
            cfg.position_x = Some(x);
            cfg.position_y = Some(y);
        }

        let pos = WindowPosition { x, y, monitor };
        if let Ok(json) = serde_json::to_string_pretty(&pos) {
            // Best-effort persistence — don't fail if directory doesn't exist yet
            if let Some(parent) = self.position_file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&self.position_file, json);
        }
    }

    /// Load persisted position from disk (used on startup).
    pub fn load_persisted_position(&self) -> Option<WindowPosition> {
        let content = std::fs::read_to_string(&self.position_file).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// Get a snapshot of the current window config.
    pub fn current_config(&self) -> WindowConfig {
        self.config.lock().unwrap().clone()
    }

    /// Check if click-through mode is active.
    pub fn is_click_through(&self) -> bool {
        self.click_through.load(Ordering::Relaxed)
    }

    /// Set click-through mode state.
    pub fn set_click_through_state(&self, enabled: bool) {
        self.click_through.store(enabled, Ordering::Relaxed);
        self.config.lock().unwrap().click_through = enabled;
    }

    /// Check if the widget is hidden due to fullscreen.
    pub fn is_hidden_for_fullscreen(&self) -> bool {
        self.hidden_for_fullscreen.load(Ordering::Relaxed)
    }

    /// Set the hidden-for-fullscreen state.
    pub fn set_hidden_for_fullscreen(&self, hidden: bool) {
        self.hidden_for_fullscreen.store(hidden, Ordering::Relaxed);
    }
}

/// Tauri-integrated window operations.
/// These functions require the Tauri AppHandle and are separated from the
/// pure-logic WindowManager to allow testing without a running Tauri app.
#[cfg(not(test))]
pub mod tauri_ops {
    use super::WindowManager;
    use std::sync::Arc;
    use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITOR_DEFAULTTONEAREST, MONITORINFO};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowLongW, GetWindowRect, SetWindowLongW,
        GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_TRANSPARENT,
    };

    /// Create or show the compact widget window with DPI-independent sizing.
    pub fn create_compact_widget(app: &AppHandle, wm: &WindowManager) -> Result<(), String> {
        let label = wm.compact_label();

        // If window already exists, just show and focus it
        if let Some(window) = app.get_webview_window(label) {
            window.show().map_err(|e| e.to_string())?;
            window.set_focus().map_err(|e| e.to_string())?;
            return Ok(());
        }

        let (width, height) = wm.compact_widget_size();
        let always_on_top = wm.is_always_on_top();

        let builder = WebviewWindowBuilder::new(app, label, WebviewUrl::default())
            .title("AI Usage Widget")
            .inner_size(width as f64, height as f64)
            .decorations(false)
            .transparent(true)
            .resizable(false)
            .skip_taskbar(true)
            .always_on_top(always_on_top);

        let window = builder.build().map_err(|e| e.to_string())?;

        // Apply DPI-independent logical size explicitly
        window
            .set_size(LogicalSize::new(width, height))
            .map_err(|e| e.to_string())?;

        // Restore persisted position if available
        if let Some(pos) = wm.load_persisted_position() {
            let _ = window.set_position(LogicalPosition::new(pos.x, pos.y));
        } else if let Some(pos) = wm.get_persisted_position() {
            let _ = window.set_position(LogicalPosition::new(pos.x, pos.y));
        }

        Ok(())
    }

    /// Toggle the dashboard window open/closed.
    pub fn toggle_dashboard(app: &AppHandle, wm: &WindowManager) -> Result<(), String> {
        let label = wm.dashboard_label();

        if wm.is_dashboard_open() {
            // Close the dashboard
            if let Some(window) = app.get_webview_window(label) {
                window.close().map_err(|e| e.to_string())?;
            }
            wm.mark_dashboard_closed();
        } else {
            // Open the dashboard
            if let Some(window) = app.get_webview_window(label) {
                window.show().map_err(|e| e.to_string())?;
                window.set_focus().map_err(|e| e.to_string())?;
            } else {
                let _window =
                    WebviewWindowBuilder::new(app, label, WebviewUrl::App("/dashboard".into()))
                        .title("AI Usage Widget - Dashboard")
                        .inner_size(900.0, 600.0)
                        .decorations(true)
                        .resizable(true)
                        .build()
                        .map_err(|e| e.to_string())?;
            }
            wm.mark_dashboard_open();
        }

        Ok(())
    }

    /// Set always-on-top state on the compact widget window via Tauri API.
    pub fn set_always_on_top(app: &AppHandle, wm: &WindowManager, enabled: bool) -> Result<(), String> {
        wm.set_always_on_top_config(enabled);

        if let Some(window) = app.get_webview_window(wm.compact_label()) {
            window
                .set_always_on_top(enabled)
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    /// Save the current window position (call on window move events).
    pub fn save_window_position(app: &AppHandle, wm: &WindowManager) -> Result<(), String> {
        if let Some(window) = app.get_webview_window(wm.compact_label()) {
            if let Ok(position) = window.outer_position() {
                // Detect which monitor the window is on
                let monitor_name = window
                    .current_monitor()
                    .ok()
                    .flatten()
                    .and_then(|m| m.name().map(|n| n.to_string()));

                wm.persist_position(position.x, position.y, monitor_name);
            }
        }

        Ok(())
    }

    /// Enable or disable click-through mode on the compact widget window.
    ///
    /// Uses Win32 `WS_EX_TRANSPARENT | WS_EX_LAYERED` extended window styles
    /// to make mouse events pass through to windows below.
    pub fn set_click_through(app: &AppHandle, wm: &WindowManager, enabled: bool) -> Result<(), String> {
        let window = app
            .get_webview_window(wm.compact_label())
            .ok_or_else(|| "Compact widget window not found".to_string())?;

        let hwnd = window.hwnd().map_err(|e| e.to_string())?;
        let hwnd = HWND(hwnd.0);

        unsafe {
            let current_style = GetWindowLongW(hwnd, GWL_EXSTYLE);

            let new_style = if enabled {
                // Add transparent + layered flags so mouse events pass through
                current_style | WS_EX_TRANSPARENT.0 as i32 | WS_EX_LAYERED.0 as i32
            } else {
                // Remove transparent flag; keep layered if it was there for other reasons
                current_style & !(WS_EX_TRANSPARENT.0 as i32)
            };

            SetWindowLongW(hwnd, GWL_EXSTYLE, new_style);
        }

        wm.set_click_through_state(enabled);
        Ok(())
    }

    /// Detect whether the current foreground window is running in fullscreen mode.
    ///
    /// Compares the foreground window's rect against the full monitor rect.
    /// Returns true if the foreground window covers the entire monitor.
    pub fn is_foreground_fullscreen() -> bool {
        unsafe {
            let fg_hwnd = GetForegroundWindow();
            if fg_hwnd.0.is_null() {
                return false;
            }

            let mut window_rect = RECT::default();
            if GetWindowRect(fg_hwnd, &mut window_rect).is_err() {
                return false;
            }

            let monitor = MonitorFromWindow(fg_hwnd, MONITOR_DEFAULTTONEAREST);
            let mut monitor_info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };

            if !GetMonitorInfoW(monitor, &mut monitor_info).as_bool() {
                return false;
            }

            let mon_rect = monitor_info.rcMonitor;

            // The foreground window is fullscreen if its rect matches/covers the monitor
            window_rect.left <= mon_rect.left
                && window_rect.top <= mon_rect.top
                && window_rect.right >= mon_rect.right
                && window_rect.bottom >= mon_rect.bottom
        }
    }

    /// Start a background loop that checks for fullscreen apps every 1 second.
    ///
    /// When a fullscreen app is detected, the widget is hidden.
    /// When fullscreen exits, the widget is shown again.
    pub fn start_fullscreen_detection_loop(app: AppHandle, wm: Arc<WindowManager>) {
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));

                let is_fullscreen = is_foreground_fullscreen();
                let was_hidden = wm.is_hidden_for_fullscreen();

                if is_fullscreen && !was_hidden {
                    // Fullscreen detected — hide the widget
                    wm.set_hidden_for_fullscreen(true);
                    if let Some(window) = app.get_webview_window(wm.compact_label()) {
                        let _ = window.hide();
                    }
                } else if !is_fullscreen && was_hidden {
                    // Fullscreen exited — show the widget again
                    wm.set_hidden_for_fullscreen(false);
                    if let Some(window) = app.get_webview_window(wm.compact_label()) {
                        let _ = window.show();
                    }
                }
            }
        });
    }

    /// Register `Win+Shift+U` global shortcut to disable click-through and focus the widget.
    ///
    /// Uses Tauri's global-shortcut plugin.
    pub fn register_click_through_shortcut(app: &AppHandle, wm: Arc<WindowManager>) -> Result<(), String> {
        use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, Code, Modifiers};

        let shortcut = Shortcut::new(
            Some(Modifiers::SUPER | Modifiers::SHIFT),
            Code::KeyU,
        );

        let app_handle = app.clone();
        let wm_clone = wm.clone();

        app.global_shortcut().on_shortcut(shortcut, move |_app, _shortcut, _event| {
            // Disable click-through mode
            if wm_clone.is_click_through() {
                let _ = set_click_through(&app_handle, &wm_clone, false);
            }

            // Bring the widget to focus
            if let Some(window) = app_handle.get_webview_window(wm_clone.compact_label()) {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }).map_err(|e| e.to_string())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_data_dir() -> PathBuf {
        std::env::temp_dir().join("ai_usage_widget_test_window")
    }

    #[test]
    fn test_new_window_manager_defaults() {
        let config = WindowConfig::default();
        let wm = WindowManager::new(config, &test_data_dir());

        assert_eq!(wm.compact_label(), "main");
        assert_eq!(wm.dashboard_label(), "dashboard");
        assert!(!wm.is_dashboard_open());
        assert!(wm.is_always_on_top());
        assert_eq!(wm.compact_widget_size(), (340, 200));
    }

    #[test]
    fn test_compact_widget_size_dpi_independent() {
        let config = WindowConfig {
            width: 340,
            height: 200,
            ..WindowConfig::default()
        };
        let wm = WindowManager::new(config, &test_data_dir());

        // Logical pixels remain the same regardless of DPI
        let (w, h) = wm.compact_widget_size();
        assert_eq!(w, 340);
        assert_eq!(h, 200);
    }

    #[test]
    fn test_toggle_dashboard_state() {
        let wm = WindowManager::new(WindowConfig::default(), &test_data_dir());

        assert!(!wm.is_dashboard_open());
        wm.mark_dashboard_open();
        assert!(wm.is_dashboard_open());
        wm.mark_dashboard_closed();
        assert!(!wm.is_dashboard_open());
    }

    #[test]
    fn test_always_on_top_config() {
        let wm = WindowManager::new(WindowConfig::default(), &test_data_dir());

        assert!(wm.is_always_on_top());
        wm.set_always_on_top_config(false);
        assert!(!wm.is_always_on_top());
        wm.set_always_on_top_config(true);
        assert!(wm.is_always_on_top());
    }

    #[test]
    fn test_persist_and_load_position() {
        let data_dir = test_data_dir().join("persist_test");
        let _ = std::fs::remove_dir_all(&data_dir);

        let wm = WindowManager::new(WindowConfig::default(), &data_dir);

        // Initially no persisted position on disk
        assert!(wm.load_persisted_position().is_none());

        // Persist a position
        wm.persist_position(100, 200, Some("Monitor1".to_string()));

        // Should be loadable from disk
        let pos = wm.load_persisted_position().unwrap();
        assert_eq!(pos.x, 100);
        assert_eq!(pos.y, 200);
        assert_eq!(pos.monitor, Some("Monitor1".to_string()));

        // Should also be in config
        let cfg_pos = wm.get_persisted_position().unwrap();
        assert_eq!(cfg_pos.x, 100);
        assert_eq!(cfg_pos.y, 200);

        // Cleanup
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn test_get_persisted_position_none_when_not_set() {
        let config = WindowConfig {
            position_x: None,
            position_y: None,
            ..WindowConfig::default()
        };
        let wm = WindowManager::new(config, &test_data_dir());
        assert!(wm.get_persisted_position().is_none());
    }

    #[test]
    fn test_get_persisted_position_some_when_set() {
        let config = WindowConfig {
            position_x: Some(50),
            position_y: Some(75),
            ..WindowConfig::default()
        };
        let wm = WindowManager::new(config, &test_data_dir());
        let pos = wm.get_persisted_position().unwrap();
        assert_eq!(pos.x, 50);
        assert_eq!(pos.y, 75);
    }

    #[test]
    fn test_current_config_snapshot() {
        let wm = WindowManager::new(WindowConfig::default(), &test_data_dir());
        let cfg = wm.current_config();
        assert_eq!(cfg.width, 340);
        assert_eq!(cfg.height, 200);
        assert!(cfg.always_on_top);
        assert!(!cfg.click_through);
    }

    #[test]
    fn test_click_through_state() {
        let wm = WindowManager::new(WindowConfig::default(), &test_data_dir());

        // Default is false (from WindowConfig default)
        assert!(!wm.is_click_through());

        wm.set_click_through_state(true);
        assert!(wm.is_click_through());
        // Also updates config
        assert!(wm.current_config().click_through);

        wm.set_click_through_state(false);
        assert!(!wm.is_click_through());
        assert!(!wm.current_config().click_through);
    }

    #[test]
    fn test_click_through_initial_from_config() {
        let config = WindowConfig {
            click_through: true,
            ..WindowConfig::default()
        };
        let wm = WindowManager::new(config, &test_data_dir());
        assert!(wm.is_click_through());
    }

    #[test]
    fn test_hidden_for_fullscreen_state() {
        let wm = WindowManager::new(WindowConfig::default(), &test_data_dir());

        // Default is not hidden
        assert!(!wm.is_hidden_for_fullscreen());

        wm.set_hidden_for_fullscreen(true);
        assert!(wm.is_hidden_for_fullscreen());

        wm.set_hidden_for_fullscreen(false);
        assert!(!wm.is_hidden_for_fullscreen());
    }
}
