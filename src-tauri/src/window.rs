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
    /// Widget height before it was collapsed, so expanding restores what the
    /// user had rather than the configured default.
    expanded_height: Mutex<Option<f64>>,
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
            expanded_height: Mutex::new(None),
        }
    }

    /// Remember the widget height to restore when it is expanded again.
    ///
    /// Collapsing an already-collapsed widget — which a webview reload does,
    /// since the frontend re-asserts the persisted state on mount — would
    /// otherwise record the 40px strip as the height to restore, and the
    /// user's real height would be gone.
    pub fn remember_expanded_height(&self, height: f64) {
        if height < Self::MIN_PERSISTABLE_HEIGHT as f64 {
            return;
        }
        *self.expanded_height.lock().unwrap() = Some(height);
    }

    /// Take the remembered height, falling back to the configured one.
    pub fn take_expanded_height(&self) -> f64 {
        self.expanded_height
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| self.config.lock().unwrap().height as f64)
    }

    /// Get the compact widget window label.
    pub fn compact_label(&self) -> &str {
        &self.compact_label
    }

    /// Get the dashboard window label.
    pub fn dashboard_label(&self) -> &str {
        &self.dashboard_label
    }

    /// Returns the logical size for the compact widget.
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

    /// Height of the widget collapsed to its header strip, logical pixels.
    pub const COLLAPSED_HEIGHT: u32 = 40;

    /// Smallest height that can be a deliberate window size rather than the
    /// widget sitting collapsed to its header strip.
    pub const MIN_PERSISTABLE_HEIGHT: u32 = 150;

    /// Whether the widget was left collapsed.
    pub fn is_collapsed(&self) -> bool {
        self.config.lock().unwrap().collapsed
    }

    /// Record whether the widget is collapsed, so it reopens the same way.
    pub fn set_collapsed(&self, collapsed: bool) {
        self.config.lock().unwrap().collapsed = collapsed;
    }

    /// The size the widget should open at.
    ///
    /// Separate from [`compact_widget_size`](Self::compact_widget_size), which
    /// is the expanded size the window remembers: building the window at the
    /// full height and letting the frontend collapse it afterwards shows the
    /// widget springing shut every launch.
    pub fn startup_widget_size(&self) -> (u32, u32) {
        let cfg = self.config.lock().unwrap();
        let height = if cfg.collapsed {
            Self::COLLAPSED_HEIGHT
        } else {
            cfg.height
        };
        (cfg.width, height)
    }

    /// Persist the window size to config.
    ///
    /// Collapsing fires a resize like any other, so without the floor the
    /// header-strip height becomes the widget's remembered size and the next
    /// launch is a 40-pixel sliver with no visible way out.
    ///
    /// The ceiling exists for the mirror-image failure: a window snapped or
    /// maximized to the display also fires a resize, and remembering the screen
    /// as the widget's own size left a letterbox stretched across the desktop.
    /// Neither extreme is a size anyone chose for a desk widget.
    pub fn persist_size(&self, width: u32, height: u32) {
        if height < Self::MIN_PERSISTABLE_HEIGHT {
            return;
        }
        if width > crate::config::MAX_SENSIBLE_WIDGET_WIDTH
            || height > crate::config::MAX_SENSIBLE_WIDGET_HEIGHT
        {
            return;
        }

        let mut cfg = self.config.lock().unwrap();
        cfg.width = width;
        cfg.height = height;
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

/// True when the process was launched by the autostart entry, which appends
/// `--minimized` (see `tray::register_autostart`) so the widget starts in the
/// tray instead of popping up at every login.
pub fn launched_minimized<S: AsRef<str>>(args: &[S]) -> bool {
    args.iter().any(|a| a.as_ref() == "--minimized")
}

/// Tauri-integrated window operations.
/// These functions require the Tauri AppHandle and are separated from the
/// pure-logic WindowManager to allow testing without a running Tauri app.
#[cfg(not(test))]
pub mod tauri_ops {
    use super::WindowManager;
    use std::sync::Arc;
    use tauri::{
        AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
    };
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowRect, IsZoomed,
        SetWindowLongW, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_TRANSPARENT,
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

        let (width, height) = wm.startup_widget_size();
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

    // The dashboard window is created by `tray::open_dashboard`, which knows
    // that the builder has to run off the main thread and that the bundled SPA
    // is served from index.html rather than a `/dashboard` route.

    /// Set always-on-top state on the compact widget window via Tauri API.
    pub fn set_always_on_top(
        app: &AppHandle,
        wm: &WindowManager,
        enabled: bool,
    ) -> Result<(), String> {
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
    pub fn set_click_through(
        app: &AppHandle,
        wm: &WindowManager,
        enabled: bool,
    ) -> Result<(), String> {
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

    /// Window classes that cover the monitor but are not fullscreen apps
    /// (desktop shell, taskbar, Start menu / notification host).
    const SHELL_WINDOW_CLASSES: [&str; 4] = [
        "Progman",
        "WorkerW",
        "Shell_TrayWnd",
        "Windows.UI.Core.CoreWindow",
    ];

    /// Read the Win32 class name of a window.
    fn window_class_name(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        let len = unsafe { GetClassNameW(hwnd, &mut buf) };
        if len <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }

    /// Detect whether the current foreground window is running in fullscreen mode.
    ///
    /// Compares the foreground window's rect against the full monitor rect.
    /// Returns true if the foreground window covers the entire monitor.
    ///
    /// A *maximized* window also covers the monitor rect (its frame even extends a
    /// few pixels past it), so `IsZoomed` is checked first — otherwise the widget
    /// would hide itself whenever any ordinary maximized window has focus. Shell
    /// windows (desktop, taskbar) are excluded for the same reason.
    pub fn is_foreground_fullscreen() -> bool {
        unsafe {
            let fg_hwnd = GetForegroundWindow();
            if fg_hwnd.0.is_null() {
                return false;
            }

            // Maximized is not fullscreen — the taskbar is still visible.
            if IsZoomed(fg_hwnd).as_bool() {
                return false;
            }

            if SHELL_WINDOW_CLASSES.contains(&window_class_name(fg_hwnd).as_str()) {
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
    pub fn register_click_through_shortcut(
        app: &AppHandle,
        wm: Arc<WindowManager>,
    ) -> Result<(), String> {
        use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

        let shortcut = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyU);

        let app_handle = app.clone();
        let wm_clone = wm.clone();

        app.global_shortcut()
            .on_shortcut(shortcut, move |_app, _shortcut, _event| {
                // Disable click-through mode
                if wm_clone.is_click_through() {
                    let _ = set_click_through(&app_handle, &wm_clone, false);
                }

                // Bring the widget to focus
                if let Some(window) = app_handle.get_webview_window(wm_clone.compact_label()) {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            })
            .map_err(|e| e.to_string())?;

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
    fn test_launched_minimized_detects_autostart_flag() {
        assert!(launched_minimized(&["app.exe", "--minimized"]));
        assert!(!launched_minimized(&["app.exe"]));
        assert!(!launched_minimized(&["app.exe", "--dashboard"]));
    }

    #[test]
    fn test_new_window_manager_defaults() {
        let config = WindowConfig::default();
        let wm = WindowManager::new(config, &test_data_dir());

        assert_eq!(wm.compact_label(), "main");
        assert_eq!(wm.dashboard_label(), "dashboard");
        assert!(!wm.is_dashboard_open());
        assert!(wm.is_always_on_top());
        assert_eq!(
            wm.compact_widget_size(),
            (
                crate::config::DEFAULT_WIDGET_WIDTH,
                crate::config::DEFAULT_WIDGET_HEIGHT
            )
        );
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
        assert_eq!(cfg.width, crate::config::DEFAULT_WIDGET_WIDTH);
        assert_eq!(cfg.height, crate::config::DEFAULT_WIDGET_HEIGHT);
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

#[cfg(test)]
mod collapse_persistence_tests {
    use super::*;

    fn manager() -> WindowManager {
        WindowManager::new(
            WindowConfig::default(),
            &std::env::temp_dir().join("ai_usage_widget_test_collapse"),
        )
    }

    #[test]
    fn test_collapsing_does_not_become_the_remembered_window_size() {
        // Collapsing resizes the window, which fires the same Resized event as
        // a user drag. Storing that height would reopen the widget as a strip
        // too short to show the control that expands it again.
        let wm = manager();
        let (width, before) = wm.compact_widget_size();

        wm.persist_size(width, 40);

        assert_eq!(wm.compact_widget_size(), (width, before));
    }

    #[test]
    fn test_a_maximized_size_is_not_remembered_as_the_widget_size() {
        // Maximizing fired a resize like any other, so the screen dimensions
        // became the widget's own. A later restore paired that width with the
        // normal height and left a letterbox stretched across the display.
        let wm = manager();
        let (width, height) = wm.compact_widget_size();

        wm.persist_size(2560, 1400);

        assert_eq!(
            wm.compact_widget_size(),
            (width, height),
            "a screen-sized resize must not become the remembered size"
        );
    }

    #[test]
    fn test_a_merely_wide_window_is_still_remembered() {
        // The ceiling must not punish someone who genuinely widened it a bit
        let wm = manager();
        wm.persist_size(700, 600);
        assert_eq!(wm.compact_widget_size(), (700, 600));
    }

    #[test]
    fn test_a_deliberate_resize_is_still_remembered() {
        let wm = manager();

        wm.persist_size(400, WindowManager::MIN_PERSISTABLE_HEIGHT);
        assert_eq!(
            wm.compact_widget_size(),
            (400, WindowManager::MIN_PERSISTABLE_HEIGHT)
        );

        wm.persist_size(420, 520);
        assert_eq!(wm.compact_widget_size(), (420, 520));
    }

    #[test]
    fn test_collapsing_twice_does_not_forget_the_real_height() {
        // A webview reload makes the frontend re-assert "collapsed" while the
        // window is already 40px. Recording that as the height to restore
        // would lose the user's size for good.
        let wm = manager();
        let (_, expanded) = wm.compact_widget_size();

        wm.remember_expanded_height(expanded as f64);
        wm.remember_expanded_height(WindowManager::COLLAPSED_HEIGHT as f64);

        assert_eq!(wm.take_expanded_height(), expanded as f64);
    }

    #[test]
    fn test_a_collapsed_widget_reopens_collapsed() {
        // Building at the full height and letting the frontend collapse it
        // shows the widget springing shut on every launch.
        let wm = manager();
        let (width, expanded) = wm.compact_widget_size();
        assert_eq!(wm.startup_widget_size(), (width, expanded));

        wm.set_collapsed(true);

        assert!(wm.is_collapsed());
        assert_eq!(
            wm.startup_widget_size(),
            (width, WindowManager::COLLAPSED_HEIGHT)
        );
        // The expanded height is remembered, not overwritten
        assert_eq!(wm.compact_widget_size(), (width, expanded));

        wm.set_collapsed(false);
        assert_eq!(wm.startup_widget_size(), (width, expanded));
    }

    #[test]
    fn test_expanding_falls_back_to_the_configured_height() {
        // Nothing was remembered, so expanding must not read the collapsed
        // strip back out of the config.
        let wm = manager();
        wm.persist_size(340, 40);

        assert_eq!(
            wm.take_expanded_height(),
            crate::config::DEFAULT_WIDGET_HEIGHT as f64
        );
    }
}
