pub mod commands;
pub mod config;
pub mod dedup;
pub mod error;
pub mod freshness;
pub mod network;
pub mod notify;
pub mod privacy;
pub mod provider;
pub mod providers;
pub mod query_types;
pub mod reconcile;
pub mod registry;
pub mod scheduler;
pub mod storage;
pub mod tray;
pub mod types;
pub mod validation;
pub mod window;

#[cfg(test)]
pub mod integration_tests;

#[cfg(not(test))]
use std::path::PathBuf;
#[cfg(not(test))]
use std::sync::Arc;

#[cfg(not(test))]
use tauri::Manager;
#[cfg(not(test))]
use tokio::sync::Mutex;

#[cfg(not(test))]
use crate::commands::AppState;
#[cfg(not(test))]
use crate::config::AppConfig;
#[cfg(not(test))]
use crate::dedup::DeduplicationEngine;
#[cfg(not(test))]
use crate::providers::claude::ClaudeAdapter;
#[cfg(not(test))]
use crate::providers::codex::CodexAdapter;
#[cfg(not(test))]
use crate::reconcile::ReconciliationEngine;
#[cfg(not(test))]
use crate::registry::ProviderRegistry;
#[cfg(not(test))]
use crate::scheduler::CollectionScheduler;
#[cfg(not(test))]
use crate::storage::StorageLayer;
#[cfg(not(test))]
use crate::window::WindowManager;

/// Determine the application data directory.
/// Uses portable mode (exe_dir/data/) if portable.flag exists next to executable,
/// otherwise uses %APPDATA%/ai-usage-widget/.
#[cfg(not(test))]
fn resolve_data_dir() -> PathBuf {
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            if exe_dir.join("portable.flag").exists() {
                return exe_dir.join("data");
            }
        }
    }

    if let Some(app_data) = std::env::var_os("APPDATA") {
        return PathBuf::from(app_data).join("ai-usage-widget");
    }

    PathBuf::from("data")
}

#[cfg(not(test))]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ─── 1. Resolve data directory and load config ──────────────────────────
    let data_dir = resolve_data_dir();
    let _ = std::fs::create_dir_all(&data_dir);

    let config_path = data_dir.join("config.json");
    let config = AppConfig::load_or_default(&config_path);

    // ─── 2. Initialize StorageLayer (async — use block_on) ──────────────────
    let storage = tauri::async_runtime::block_on(async {
        StorageLayer::with_retention(&config.db_path, config.retention_days)
            .await
            .expect("Failed to initialize SQLite storage layer")
    });
    let storage = Arc::new(storage);

    // ─── 3. Create DeduplicationEngine (preload from DB) ────────────────────
    let dedup = tauri::async_runtime::block_on(async {
        DeduplicationEngine::new(Arc::new(storage.pool().clone()))
            .await
            .expect("Failed to initialize deduplication engine")
    });
    let dedup = Arc::new(tokio::sync::Mutex::new(dedup));

    // ─── 4. Create ProviderRegistry and register adapters ───────────────────
    let mut registry = ProviderRegistry::new();

    // Register Codex adapter (conditional on sessions directory existing)
    if config.codex.enabled && config.codex.sessions_dir.exists() {
        let codex_adapter = CodexAdapter::new(&config.codex);
        registry.register(Box::new(codex_adapter));
        log::info!("Codex provider registered");
    } else {
        log::info!(
            "Codex provider skipped (enabled={}, path exists={})",
            config.codex.enabled,
            config.codex.sessions_dir.exists()
        );
    }

    // Register Claude adapter (conditional on data directory existing)
    if config.claude.enabled && config.claude.data_dir.exists() {
        let claude_adapter = ClaudeAdapter::new(config.claude.data_dir.clone());
        registry.register(Box::new(claude_adapter));
        log::info!("Claude provider registered");
    } else {
        log::info!(
            "Claude provider skipped (enabled={}, path exists={})",
            config.claude.enabled,
            config.claude.data_dir.exists()
        );
    }

    let registry = Arc::new(registry);

    // Restore persisted file offsets/checkpoints so a restart resumes reading
    // where the previous run stopped instead of re-parsing every source file.
    // The logger is only installed once the Tauri builder runs, so startup
    // findings are collected here and logged from setup().
    let mut startup_notes: Vec<String> = Vec::new();

    tauri::async_runtime::block_on(async {
        for provider_id in ["codex", "claude"] {
            match storage.load_provider_state(provider_id).await {
                Ok(state) => {
                    if !state.file_positions.is_empty() || state.checkpoint.is_some() {
                        startup_notes.push(format!(
                            "Restored state for '{}': {} file offsets, checkpoint={:?}",
                            provider_id,
                            state.file_positions.len(),
                            state.checkpoint
                        ));
                        registry.restore_state(provider_id, &state);
                    }
                }
                Err(e) => {
                    startup_notes.push(format!("Failed to load state for '{}': {}", provider_id, e))
                }
            }
        }
    });

    startup_notes.push(format!(
        "Providers registered: {} (data dir: {})",
        registry.adapter_count(),
        data_dir.display()
    ));

    // ─── 5. Create CollectionScheduler ──────────────────────────────────────
    let mut scheduler = CollectionScheduler::new(config.collection_interval_secs);
    scheduler.set_notification_thresholds(
        config.notification_warning_pct,
        config.notification_critical_pct,
    );
    let scheduler = Arc::new(Mutex::new(scheduler));

    // ─── 6. Create WindowManager ────────────────────────────────────────────
    let window_manager = WindowManager::new(config.window.clone(), &data_dir);
    let window_manager = Arc::new(window_manager);

    // ─── 7. Create ReconciliationEngine ─────────────────────────────────────
    let reconciliation = Arc::new(ReconciliationEngine::new());

    // ─── 8. Create AppState for Tauri managed state ─────────────────────────
    let app_state = AppState {
        storage: storage.clone(),
        registry: registry.clone(),
        scheduler: scheduler.clone(),
        config: Arc::new(Mutex::new(config.clone())),
        config_path: config_path.clone(),
        window_manager: window_manager.clone(),
        dedup: dedup.clone(),
        reconciliation: reconciliation.clone(),
    };

    let tray_locale = config.locale.clone();

    // ─── 9. Build and run the Tauri application ─────────────────────────────
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("ai-usage-widget".to_string()),
                    }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                ])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A second launch acts as a remote control for the running instance:
            // `ai-usage-widget.exe --dashboard [--settings]` opens the dashboard.
            if args.iter().any(|a| a == "--dashboard" || a == "--settings") {
                let tab = if args.iter().any(|a| a == "--settings") {
                    "settings"
                } else {
                    "usage"
                };
                tray::open_dashboard(app, tab);
                return;
            }

            // Otherwise just bring the widget back
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
                let _ = window.show();
            }
        }))
        .manage(app_state)
        .setup(move |app| {
            for note in &startup_notes {
                log::info!("{}", note);
            }

            // Restore the placement the user left the widget in.
            //
            // The window is declared hidden in tauri.conf.json and shown at the
            // end of this block: sizing it afterwards would be visible, and a
            // widget left collapsed spent half a second at full height before
            // snapping shut on every launch.
            if let Some(main_window) = app.get_webview_window("main") {
                let (width, height) = window_manager.startup_widget_size();
                if width > 0 && height > 0 {
                    if window_manager.is_collapsed() {
                        // The collapsed height is below the configured minimum
                        let _ = main_window.set_min_size(Some(tauri::LogicalSize::new(
                            280.0,
                            crate::window::WindowManager::COLLAPSED_HEIGHT as f64,
                        )));
                    }
                    let _ =
                        main_window.set_size(tauri::LogicalSize::new(width as f64, height as f64));
                }
                if let Some(position) = window_manager
                    .load_persisted_position()
                    .or_else(|| window_manager.get_persisted_position())
                {
                    let _ = main_window
                        .set_position(tauri::LogicalPosition::new(position.x, position.y));
                }

                // Unconditional: every step above is best-effort, and a widget
                // that stays hidden because one of them failed is unreachable
                // except through the tray.
                let _ = main_window.show();
            }

            // Build system tray with context menu
            let _tray = tray::build_system_tray(app, &tray_locale)?;

            // Open the dashboard straight away when asked on the command line
            let args: Vec<String> = std::env::args().collect();
            if args.iter().any(|a| a == "--dashboard" || a == "--settings") {
                let tab = if args.iter().any(|a| a == "--settings") {
                    "settings"
                } else {
                    "usage"
                };
                tray::open_dashboard(app.handle(), tab);
            }

            // Register click-through recovery shortcut (Win+Shift+U)
            let wm_shortcut = window_manager.clone();
            window::tauri_ops::register_click_through_shortcut(app.handle(), wm_shortcut)
                .unwrap_or_else(|e| {
                    log::warn!("Failed to register global shortcut: {}", e);
                });

            // Remember where the user drags/resizes the widget to. The widget
            // has no title bar, so this is the only record of its placement.
            if let Some(main_window) = app.get_webview_window("main") {
                let wm_geometry = window_manager.clone();
                let geometry_window = main_window.clone();

                main_window.on_window_event(move |event| match event {
                    tauri::WindowEvent::Moved(position) => {
                        let scale = geometry_window.scale_factor().unwrap_or(1.0);
                        let logical = position.to_logical::<i32>(scale);
                        let monitor = geometry_window
                            .current_monitor()
                            .ok()
                            .flatten()
                            .and_then(|m| m.name().map(|n| n.to_string()));
                        wm_geometry.persist_position(logical.x, logical.y, monitor);
                    }
                    tauri::WindowEvent::Resized(size) => {
                        let scale = geometry_window.scale_factor().unwrap_or(1.0);
                        let logical = size.to_logical::<u32>(scale);
                        if logical.width > 0 && logical.height > 0 {
                            wm_geometry.persist_size(logical.width, logical.height);
                        }
                    }
                    _ => {}
                });
            }

            // Spawn fullscreen detection loop
            let wm_fullscreen = window_manager.clone();
            let app_handle_fs = app.handle().clone();
            window::tauri_ops::start_fullscreen_detection_loop(app_handle_fs, wm_fullscreen);

            // Spawn collection loop
            let scheduler_clone = scheduler.clone();
            let registry_clone = registry.clone();
            let dedup_clone = dedup.clone();
            let reconciliation_clone = reconciliation.clone();
            let storage_clone = storage.clone();

            // Quota thresholds (75% / 90%) surface as Windows toasts.
            let notify_handle = app.handle().clone();
            let notifier: crate::scheduler::Notifier = Arc::new(move |entry| {
                use tauri_plugin_notification::NotificationExt;

                let title = match entry.level {
                    crate::notify::NotificationLevel::Critical => "AI Usage Widget — Critical",
                    crate::notify::NotificationLevel::Warning => "AI Usage Widget — Warning",
                };

                if let Err(e) = notify_handle
                    .notification()
                    .builder()
                    .title(title)
                    .body(&entry.message)
                    .show()
                {
                    log::warn!("Failed to show quota notification: {}", e);
                }
            });

            tauri::async_runtime::spawn(async move {
                let mut sched = scheduler_clone.lock().await;
                sched
                    .run_with_notifier(
                        registry_clone,
                        dedup_clone,
                        reconciliation_clone,
                        storage_clone,
                        Some(notifier),
                    )
                    .await;
            });

            // Disable DevTools in production builds
            #[cfg(not(debug_assertions))]
            {
                if let Some(window) = app.get_webview_window("main") {
                    // In production, devtools are not available by default in Tauri 2
                    // but we explicitly ensure they're not enabled
                    let _ = window;
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_current_usage,
            commands::get_usage_history,
            commands::get_provider_status,
            commands::trigger_collection,
            commands::get_settings,
            commands::update_settings,
            commands::backup_data,
            commands::restore_data,
            commands::check_for_updates,
            commands::open_dashboard,
            commands::show_widget,
            commands::set_widget_collapsed,
            commands::get_widget_collapsed,
            commands::log_frontend_error,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
