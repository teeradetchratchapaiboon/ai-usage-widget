//! System tray integration and Windows autostart management.
//!
//! Provides:
//! - System tray context menu (Show Widget, Dashboard, Collect Now, Language, Settings, Quit)
//! - Autostart registration via Windows Registry (`HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run`)
//! - Single-instance enforcement is handled by `tauri-plugin-single-instance` in lib.rs

use std::path::Path;

/// Registry key path for Windows autostart entries.
const AUTOSTART_REGISTRY_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";

/// Registry value name for this application.
const AUTOSTART_VALUE_NAME: &str = "AIUsageWidget";

/// Tray menu item IDs.
pub mod menu_ids {
    pub const SHOW_WIDGET: &str = "show_widget";
    pub const SHOW_DASHBOARD: &str = "show_dashboard";
    pub const COLLECT_NOW: &str = "collect_now";
    pub const LANGUAGE: &str = "language";
    pub const SETTINGS: &str = "settings";
    pub const QUIT: &str = "quit";
}

/// Errors that can occur during autostart operations.
#[derive(Debug, thiserror::Error)]
pub enum AutostartError {
    #[error("Failed to open registry key: {0}")]
    RegistryOpen(String),
    #[error("Failed to set registry value: {0}")]
    RegistrySet(String),
    #[error("Failed to delete registry value: {0}")]
    RegistryDelete(String),
    #[error("Failed to query registry value: {0}")]
    RegistryQuery(String),
}

/// Encode a Rust string as null-terminated UTF-16LE bytes suitable for REG_SZ.
fn string_to_reg_sz_bytes(s: &str) -> Vec<u8> {
    let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let mut bytes = Vec::with_capacity(wide.len() * 2);
    for w in wide {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    bytes
}

/// Register the application for autostart via Windows Registry.
///
/// Writes the executable path to `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run\AIUsageWidget`.
/// The `--minimized` argument is appended so the widget starts hidden in the tray.
#[cfg(windows)]
pub fn register_autostart(exe_path: &Path) -> Result<(), AutostartError> {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    let key_path = HSTRING::from(AUTOSTART_REGISTRY_KEY);
    let value_name = HSTRING::from(AUTOSTART_VALUE_NAME);

    // Build the command string: "path\to\exe" --minimized
    let exe_str = exe_path.to_string_lossy();
    let command = format!("\"{}\" --minimized", exe_str);
    let bytes = string_to_reg_sz_bytes(&command);

    unsafe {
        let mut hkey = HKEY::default();
        let result = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &key_path,
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        );

        if result.is_err() {
            return Err(AutostartError::RegistryOpen(format!(
                "RegCreateKeyExW failed: {:?}",
                result
            )));
        }

        let set_result = RegSetValueExW(
            hkey,
            &value_name,
            Some(0),
            REG_SZ,
            Some(&bytes),
        );

        let _ = RegCloseKey(hkey);

        if set_result.is_err() {
            return Err(AutostartError::RegistrySet(format!(
                "RegSetValueExW failed: {:?}",
                set_result
            )));
        }
    }

    Ok(())
}

/// Unregister the application from autostart by removing the Registry value.
#[cfg(windows)]
pub fn unregister_autostart() -> Result<(), AutostartError> {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE,
    };

    let key_path = HSTRING::from(AUTOSTART_REGISTRY_KEY);
    let value_name = HSTRING::from(AUTOSTART_VALUE_NAME);

    unsafe {
        let mut hkey = HKEY::default();
        let open_result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &key_path,
            Some(0),
            KEY_WRITE,
            &mut hkey,
        );

        if open_result.is_err() {
            // Key doesn't exist or can't be opened — already unregistered
            return Ok(());
        }

        let del_result = RegDeleteValueW(hkey, &value_name);
        let _ = RegCloseKey(hkey);

        if del_result.is_err() {
            // Value doesn't exist — that's fine, already unregistered
            return Ok(());
        }
    }

    Ok(())
}

/// Check if the application is currently registered for autostart.
#[cfg(windows)]
pub fn is_autostart_enabled() -> Result<bool, AutostartError> {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };

    let key_path = HSTRING::from(AUTOSTART_REGISTRY_KEY);
    let value_name = HSTRING::from(AUTOSTART_VALUE_NAME);

    unsafe {
        let mut hkey = HKEY::default();
        let open_result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &key_path,
            Some(0),
            KEY_READ,
            &mut hkey,
        );

        if open_result.is_err() {
            return Ok(false);
        }

        let query_result = RegQueryValueExW(
            hkey,
            &value_name,
            None,
            None,
            None,
            None,
        );

        let _ = RegCloseKey(hkey);

        Ok(query_result.is_ok())
    }
}

/// Build the system tray context menu and tray icon for the application.
///
/// Menu items:
/// - Show Widget: shows and focuses the compact widget window
/// - Dashboard: toggles the dashboard window
/// - Collect Now: triggers immediate data collection
/// - Language: TH/EN: switches locale
/// - Settings: opens settings panel
/// - Quit: exits the application
///
/// This function is intended to be called during Tauri app setup.
#[cfg(not(test))]
pub fn build_system_tray(
    app: &tauri::App,
) -> Result<tauri::tray::TrayIcon, Box<dyn std::error::Error>> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::TrayIconBuilder,
    };

    let show_widget =
        MenuItem::with_id(app, menu_ids::SHOW_WIDGET, "Show Widget", true, None::<&str>)?;
    let show_dashboard =
        MenuItem::with_id(app, menu_ids::SHOW_DASHBOARD, "Dashboard", true, None::<&str>)?;
    let collect_now =
        MenuItem::with_id(app, menu_ids::COLLECT_NOW, "Collect Now", true, None::<&str>)?;
    let language =
        MenuItem::with_id(app, menu_ids::LANGUAGE, "Language: TH/EN", true, None::<&str>)?;
    let settings =
        MenuItem::with_id(app, menu_ids::SETTINGS, "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, menu_ids::QUIT, "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show_widget,
            &show_dashboard,
            &collect_now,
            &language,
            &settings,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::new();

    // Without an explicit icon the Shell_NotifyIcon entry renders blank on Windows,
    // which makes the tray-only widget unreachable. Reuse the bundled window icon.
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    let tray = builder
        .menu(&menu)
        .tooltip("AI Usage Widget")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
            use tauri::Manager;

            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .on_menu_event(|app, event| {
            handle_tray_menu_event(app, event.id.as_ref());
        })
        .build(app)?;

    Ok(tray)
}

/// Show the dashboard window on the requested tab, creating it on first use.
///
/// The tab is delivered as an event; the window is created hidden-then-shown so
/// the tab is already set when the view first paints.
#[cfg(not(test))]
pub fn open_dashboard(app: &tauri::AppHandle, tab: &str) {
    use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

    if let Some(window) = app.get_webview_window("dashboard") {
        let _ = window.show();
        let _ = window.set_focus();
        // The view is already mounted, so the tab arrives as an event.
        let _ = window.emit("dashboard-tab", tab.to_string());
        return;
    }

    // First open: inject the tab before the app boots. An event would fire
    // before the webview registered its listener, and a query string on the
    // asset URL does not resolve to the bundled index.html.
    let tab_script = format!(
        "window.__DASHBOARD_TAB__ = {};",
        serde_json::to_string(tab).unwrap_or_else(|_| "\"usage\"".to_string())
    );

    match WebviewWindowBuilder::new(app, "dashboard", WebviewUrl::App("index.html".into()))
        .initialization_script(tab_script)
        .title("AI Usage Widget — Dashboard")
        .inner_size(920.0, 620.0)
        .min_inner_size(720.0, 480.0)
        .decorations(true)
        .resizable(true)
        .build()
    {
        Ok(window) => {
            log::info!(
                "Dashboard window created (url={:?})",
                window.url().map(|u| u.to_string())
            );
            let _ = window.show();
            let _ = window.set_focus();
        }
        Err(e) => log::error!("Failed to create dashboard window: {}", e),
    }
}

/// Handle tray menu item clicks.
///
/// Dispatched from the tray icon's on_menu_event callback.
#[cfg(not(test))]
fn handle_tray_menu_event(app: &tauri::AppHandle, menu_id: &str) {
    use tauri::{Emitter, Manager};

    match menu_id {
        menu_ids::SHOW_WIDGET => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        menu_ids::SHOW_DASHBOARD => {
            open_dashboard(app, "usage");
        }
        menu_ids::COLLECT_NOW => {
            // Emit an event that the frontend turns into a trigger_collection call
            let _ = app.emit("collect-now", ());
        }
        menu_ids::LANGUAGE => {
            // Emit a locale toggle event for the frontend
            let _ = app.emit("toggle-locale", ());
        }
        menu_ids::SETTINGS => {
            // Settings live in the dashboard window — open it on the settings tab
            open_dashboard(app, "settings");
        }
        menu_ids::QUIT => {
            app.exit(0);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_menu_ids_are_distinct() {
        let ids = [
            menu_ids::SHOW_WIDGET,
            menu_ids::SHOW_DASHBOARD,
            menu_ids::COLLECT_NOW,
            menu_ids::LANGUAGE,
            menu_ids::SETTINGS,
            menu_ids::QUIT,
        ];

        // All IDs should be unique
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                assert_ne!(ids[i], ids[j], "Menu IDs must be unique");
            }
        }
    }

    #[test]
    fn test_autostart_registry_constants() {
        assert_eq!(
            AUTOSTART_REGISTRY_KEY,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run"
        );
        assert_eq!(AUTOSTART_VALUE_NAME, "AIUsageWidget");
    }

    #[test]
    fn test_string_to_reg_sz_bytes() {
        let result = string_to_reg_sz_bytes("AB");
        // 'A' = 0x41, 'B' = 0x42, null = 0x00
        // UTF-16LE: [0x41, 0x00, 0x42, 0x00, 0x00, 0x00]
        assert_eq!(result, vec![0x41, 0x00, 0x42, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_string_to_reg_sz_bytes_empty() {
        let result = string_to_reg_sz_bytes("");
        // Just null terminator
        assert_eq!(result, vec![0x00, 0x00]);
    }

    // Writes to the real HKCU\...\Run key of whoever runs the suite, so it is
    // opt-in: `cargo test -- --ignored autostart`.
    #[test]
    #[ignore = "mutates the current user's Windows autostart registry key"]
    fn test_autostart_round_trip() {
        // Register autostart with a fake path
        let fake_exe = PathBuf::from(r"C:\Test\AIUsageWidget.exe");

        // Register
        let reg_result = register_autostart(&fake_exe);
        assert!(reg_result.is_ok(), "register_autostart should succeed");

        // Verify it's enabled
        let enabled = is_autostart_enabled().unwrap();
        assert!(enabled, "autostart should be enabled after registration");

        // Unregister
        let unreg_result = unregister_autostart();
        assert!(unreg_result.is_ok(), "unregister_autostart should succeed");

        // Verify it's disabled
        let disabled = !is_autostart_enabled().unwrap();
        assert!(disabled, "autostart should be disabled after unregistration");
    }

    #[test]
    fn test_unregister_when_not_registered() {
        // Ensure clean state first
        let _ = unregister_autostart();

        // Unregistering when not registered should be idempotent (no error)
        let result = unregister_autostart();
        assert!(result.is_ok());
    }
}
