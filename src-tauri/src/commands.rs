use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::config::AppConfig;
use crate::freshness::QuotaTiming;
use crate::provider::ProviderSummary;
use crate::query_types::{Granularity, TimeRange, UsageRecord, UsageSummary};
use crate::registry::ProviderRegistry;
use crate::storage::StorageLayer;
use crate::validation::{validate_settings, validate_time_range, PartialSettings};

// ─── Managed Application State ─────────────────────────────────────────────────

/// Shared application state managed by Tauri.
///
/// Holds references to all core subsystems that IPC commands need to access.
pub struct AppState {
    pub storage: Arc<StorageLayer>,
    pub registry: Arc<ProviderRegistry>,
    /// Lock-free control of the running collection loop (see `SchedulerHandle`).
    pub scheduler: crate::scheduler::SchedulerHandle,
    pub config: Arc<Mutex<AppConfig>>,
    /// Path to the config file on disk for persisting changes.
    pub config_path: PathBuf,
    /// Window state, so settings changes reach the live widget.
    pub window_manager: Arc<crate::window::WindowManager>,
    /// Dedup/reconcile engines, so manual collection stores through the same
    /// pipeline the scheduler uses.
    pub dedup: Arc<Mutex<crate::dedup::DeduplicationEngine>>,
    pub reconciliation: Arc<crate::reconcile::ReconciliationEngine>,
}

// ─── Response Types ─────────────────────────────────────────────────────────────

/// Status information for a single registered provider.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatusResponse {
    pub provider_id: String,
    pub display_name: String,
    pub is_available: bool,
    /// Most recent activity the provider itself reports (RFC 3339).
    ///
    /// Renamed from `last_collection`, which it never was: nothing here
    /// describes a collection attempt, and the old name invited reading it as
    /// the age of the quota — a question `quota_*_observed_at` answers.
    pub last_activity: Option<String>,
    pub errors: Vec<String>,
    /// Quota consumed, in percent, for providers that publish quota data.
    pub quota_fast_pct: Option<f64>,
    pub quota_standard_pct: Option<f64>,
    pub quota_excess_pct: Option<f64>,
    /// Tokens reported by the provider itself for today, when available.
    pub tokens_today: Option<u64>,
    /// When the five-hour window resets (RFC 3339), when known.
    pub quota_fast_resets_at: Option<String>,
    /// When the weekly window resets (RFC 3339), when known.
    pub quota_weekly_resets_at: Option<String>,
    /// True when the reset times above were derived, not published.
    ///
    /// Confidence in the *reset time* only. It is independent of freshness,
    /// which is confidence in the *percentage*: Codex publishes an exact reset
    /// for a number that may be two days old, Claude infers a reset for one
    /// written a minute ago.
    pub quota_resets_estimated: bool,
    /// When the source record supplying each percentage was written (RFC 3339).
    pub quota_fast_observed_at: Option<String>,
    pub quota_weekly_observed_at: Option<String>,
    /// Per-window freshness: `fresh` | `aging` | `stale` | `expired` | `unknown`.
    pub quota_fast_freshness: String,
    pub quota_weekly_freshness: String,
    /// Age of each reading in whole seconds, when it can be determined.
    pub quota_fast_age_secs: Option<i64>,
    pub quota_weekly_age_secs: Option<i64>,
}

/// Result of triggering an immediate collection cycle.
#[derive(Debug, Clone, Serialize)]
pub struct CollectionResponse {
    pub events_collected: u32,
    pub providers_collected: u32,
    pub errors: Vec<String>,
}

/// Application settings exposed to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub collection_interval_secs: u32,
    pub locale: String,
    pub notification_warning_pct: f64,
    pub notification_critical_pct: f64,
    pub autostart: bool,
    pub always_on_top: bool,
    pub click_through: bool,
    /// Directory holding the database and config, for display in settings.
    #[serde(default)]
    pub data_dir: String,
}

/// Information about an available update.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub download_url: String,
    pub release_notes: Option<String>,
}

// ─── Tauri Commands ─────────────────────────────────────────────────────────────

/// Query the storage layer for today's/this week's usage summary.
#[tauri::command]
pub async fn get_current_usage(state: tauri::State<'_, AppState>) -> Result<UsageSummary, String> {
    state
        .storage
        .get_current_summary()
        .await
        .map_err(|e| format!("Failed to get current usage: {}", e))
}

/// Validated time range query with ISO 8601 parsing and granularity selection.
#[tauri::command]
pub async fn get_usage_history(
    start: String,
    end: String,
    granularity: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<UsageRecord>, String> {
    // Parse ISO 8601 timestamps
    let start_dt = DateTime::parse_from_rfc3339(&start)
        .map_err(|e| format!("Invalid start timestamp '{}': {}", start, e))?
        .with_timezone(&chrono::Utc);

    let end_dt = DateTime::parse_from_rfc3339(&end)
        .map_err(|e| format!("Invalid end timestamp '{}': {}", end, e))?
        .with_timezone(&chrono::Utc);

    // Parse granularity
    let gran = match granularity.to_lowercase().as_str() {
        "hourly" => Granularity::Hourly,
        "daily" => Granularity::Daily,
        "weekly" => Granularity::Weekly,
        "monthly" => Granularity::Monthly,
        other => {
            return Err(format!(
                "Invalid granularity '{}': must be one of hourly, daily, weekly, monthly",
                other
            ))
        }
    };

    // Construct and validate time range
    let range = TimeRange {
        start: start_dt,
        end: end_dt,
    };

    validate_time_range(&range).map_err(|e| format!("Time range validation failed: {}", e))?;

    // Query storage
    state
        .storage
        .get_history(&range, gran)
        .await
        .map_err(|e| format!("Failed to get usage history: {}", e))
}

/// Per-window timings taken straight off a summary.
///
/// Built in one place so the widget and the dashboard cannot disagree about
/// which reset time belongs to which observation.
pub fn quota_timings(summary: &ProviderSummary) -> (QuotaTiming, QuotaTiming) {
    (
        QuotaTiming {
            observed_at: summary.quota_observed.fast_hours,
            resets_at: summary.quota_resets.fast_hours,
        },
        QuotaTiming {
            observed_at: summary.quota_observed.weekly,
            resets_at: summary.quota_resets.weekly,
        },
    )
}

/// Convert one provider summary into its wire representation.
///
/// Split out of the command so the mapping is reachable from tests: a field
/// hardcoded here rather than read off the summary — `quota_resets_estimated`
/// and the freshness fields especially — would otherwise pass every test in
/// the suite while telling the UI the wrong thing.
///
/// `now` is a parameter so tests can pin the clock. It is used *only* to
/// classify freshness; it never becomes an observation timestamp.
pub fn provider_status_response_at(
    summary: ProviderSummary,
    now: DateTime<Utc>,
) -> ProviderStatusResponse {
    let (fast, weekly) = quota_timings(&summary);

    ProviderStatusResponse {
        provider_id: summary.provider_id,
        display_name: summary.display_name,
        is_available: summary.is_available,
        last_activity: summary.last_activity.map(|dt| dt.to_rfc3339()),
        errors: Vec::new(),
        quota_fast_pct: summary.quota.as_ref().and_then(|q| q.fast_hours_pct),
        quota_standard_pct: summary.quota.as_ref().and_then(|q| q.standard_pct),
        quota_excess_pct: summary.quota.as_ref().and_then(|q| q.excess_pct),
        tokens_today: summary.tokens_today,
        quota_fast_resets_at: summary.quota_resets.fast_hours.map(|dt| dt.to_rfc3339()),
        quota_weekly_resets_at: summary.quota_resets.weekly.map(|dt| dt.to_rfc3339()),
        quota_resets_estimated: summary.quota_resets.estimated,
        quota_fast_observed_at: fast.observed_at.map(|dt| dt.to_rfc3339()),
        quota_weekly_observed_at: weekly.observed_at.map(|dt| dt.to_rfc3339()),
        quota_fast_freshness: fast.freshness(now).as_str().to_string(),
        quota_weekly_freshness: weekly.freshness(now).as_str().to_string(),
        quota_fast_age_secs: fast.age_seconds(now),
        quota_weekly_age_secs: weekly.age_seconds(now),
    }
}

/// [`provider_status_response_at`] against the wall clock.
pub fn provider_status_response(summary: ProviderSummary) -> ProviderStatusResponse {
    provider_status_response_at(summary, Utc::now())
}

/// Return status of all registered providers.
#[tauri::command]
pub async fn get_provider_status(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProviderStatusResponse>, String> {
    Ok(state
        .registry
        .get_all_summaries()
        .into_iter()
        .map(provider_status_response)
        .collect())
}

/// Open (or focus) the dashboard window on the given tab.
///
/// Same entry point the tray menu uses, exposed so the widget can offer it too.
#[cfg(not(test))]
#[tauri::command]
pub fn open_dashboard(app: tauri::AppHandle, tab: Option<String>) -> Result<(), String> {
    let tab = tab.unwrap_or_else(|| "usage".to_string());
    crate::tray::open_dashboard(&app, &tab);
    Ok(())
}

/// Leave dashboard mode: close the dashboard and bring the widget back.
///
/// The two views are exclusive — the widget skips the taskbar, so leaving it
/// behind a full dashboard would make it unreachable except from the tray.
#[cfg(not(test))]
#[tauri::command]
pub fn show_widget(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;

    if let Some(dashboard) = app.get_webview_window("dashboard") {
        dashboard.close().map_err(|e| e.to_string())?;
    }

    // Absent window is still an error worth reporting, but the show itself
    // goes through the shared helper so the visibility event cannot be missed.
    app.get_webview_window("main")
        .ok_or_else(|| "widget window is gone".to_string())?;
    crate::tray::show_widget(&app);

    Ok(())
}

/// Whether the widget was left collapsed when it was last closed.
///
/// The frontend owns the collapsed flag at runtime but cannot remember it
/// across launches, so it asks for the persisted value on mount.
#[cfg(not(test))]
#[tauri::command]
pub async fn get_widget_collapsed(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(state.window_manager.is_collapsed())
}

/// Collapse the widget to its header strip, or restore its previous height.
///
/// Resizing lives on this side because the collapsed height is below the
/// window's minimum, which has to be lifted for the duration.
#[cfg(not(test))]
#[tauri::command]
pub async fn set_widget_collapsed(
    app: tauri::AppHandle,
    collapsed: bool,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    use crate::window::WindowManager;
    use tauri::{LogicalSize, Manager};

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "widget window is gone".to_string())?;

    // Held across the whole toggle so two of them cannot interleave: without
    // it, both could read the pre-resize size and the writes could land in the
    // opposite order to the resizes, leaving the config disagreeing with the
    // window it describes.
    let mut config = state.config.lock().await;

    // Persisted first, and nothing is touched if it fails. The frontend rolls
    // back its own flag on an error but cannot resize the window, so a failure
    // after the resize would strand an expanded UI inside a 40px strip —
    // exactly what this command exists to avoid.
    //
    // Written on every toggle rather than at shutdown: the app exits via the
    // tray and is routinely killed, and a preference that survives only a
    // clean exit is not a preference.
    let previous = config.window.collapsed;
    config.window.collapsed = collapsed;
    if let Err(e) = config.save_to_file(&state.config_path) {
        config.window.collapsed = previous;
        return Err(format!("Failed to persist collapsed state: {}", e));
    }
    state.window_manager.set_collapsed(collapsed);

    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let current = window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(scale);

    let (min_height, height) = if collapsed {
        state
            .window_manager
            .remember_expanded_height(current.height);
        (
            WindowManager::COLLAPSED_HEIGHT as f64,
            WindowManager::COLLAPSED_HEIGHT as f64,
        )
    } else {
        (
            WindowManager::MIN_PERSISTABLE_HEIGHT as f64,
            state.window_manager.take_expanded_height(),
        )
    };

    // The remembered width, not whatever the window happens to be. Carrying the
    // live width forward re-applied a maximized one to a normal-height window,
    // which is how the widget ended up a letterbox across the screen.
    let width = state
        .window_manager
        .current_config()
        .width
        .min(crate::config::MAX_SENSIBLE_WIDGET_WIDTH) as f64;

    // Order matters: the minimum has to allow the new height before we ask for it
    window
        .set_min_size(Some(LogicalSize::new(280.0, min_height)))
        .map_err(|e| e.to_string())?;
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Hide the widget to the system tray.
///
/// Hidden, not closed: the collector keeps running and the tray's "Show
/// Widget" item brings it straight back. The window skips the taskbar, so the
/// tray is deliberately the only way back — the same path the tray menu and
/// the dashboard already use.
#[cfg(not(test))]
#[tauri::command]
pub fn hide_widget(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;

    app.get_webview_window("main")
        .ok_or_else(|| "widget window is gone".to_string())?;
    crate::tray::hide_widget(&app);

    Ok(())
}

/// Record an uncaught frontend error in the application log.
///
/// Release builds have no devtools, so without this a webview exception is
/// invisible: the window just renders blank.
#[tauri::command]
pub fn log_frontend_error(window: String, message: String) {
    log::error!("[webview:{}] {}", window, message);
}

/// Trigger an immediate collection cycle.
#[tauri::command]
pub async fn trigger_collection(
    state: tauri::State<'_, AppState>,
) -> Result<CollectionResponse, String> {
    let outcomes = state.registry.collect_all();

    let mut all_events = Vec::new();
    let mut providers_collected: u32 = 0;
    let mut errors: Vec<String> = Vec::new();

    for outcome in outcomes {
        match outcome {
            crate::registry::ProviderCollectionOutcome::Success {
                provider_id,
                result,
            } => {
                providers_collected += 1;
                log::info!(
                    "Trigger collection: provider '{}' returned {} events",
                    provider_id,
                    result.events.len()
                );
                all_events.extend(result.events);
            }
            crate::registry::ProviderCollectionOutcome::Failed { provider_id, error } => {
                errors.push(format!("Provider '{}': {}", provider_id, error));
            }
        }
    }

    // Store through the same dedup -> reconcile -> store pipeline as the
    // scheduler, so "Collect Now" actually persists what it collected.
    let events_collected = if all_events.is_empty() {
        0
    } else {
        match crate::scheduler::process_events(
            all_events,
            &state.dedup,
            &state.reconciliation,
            &state.storage,
        )
        .await
        {
            Ok(stored) => stored as u32,
            Err(e) => {
                errors.push(format!("Failed to store collected events: {}", e));
                0
            }
        }
    };

    // Persist the advanced file offsets too
    for (provider_id, provider_state) in state.registry.export_states() {
        if let Err(e) = state
            .storage
            .save_provider_state(&provider_id, &provider_state)
            .await
        {
            log::warn!("Failed to persist state for '{}': {}", provider_id, e);
        }
    }

    Ok(CollectionResponse {
        events_collected,
        providers_collected,
        errors,
    })
}

/// Read current application settings.
#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, AppState>) -> Result<AppSettings, String> {
    let config = state.config.lock().await;

    Ok(AppSettings {
        collection_interval_secs: config.collection_interval_secs,
        locale: config.locale.clone(),
        notification_warning_pct: config.notification_warning_pct,
        notification_critical_pct: config.notification_critical_pct,
        autostart: crate::tray::is_autostart_enabled().unwrap_or(false),
        always_on_top: config.window.always_on_top,
        click_through: config.window.click_through,
        data_dir: state
            .config_path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    })
}

/// Validate and persist settings changes.
#[tauri::command]
pub async fn update_settings(
    settings: PartialSettings,
    #[cfg_attr(test, allow(unused_variables))] app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    // Validate inputs before applying
    validate_settings(&settings).map_err(|e| format!("Settings validation failed: {}", e))?;

    // Invariant: nothing below awaits while this guard is held, so a slow or
    // stuck subsystem cannot block every other user of the config.
    let mut config = state.config.lock().await;

    // Apply partial updates
    if let Some(ref locale) = settings.locale {
        config.locale = locale.clone();
    }
    if let Some(interval) = settings.collection_interval_secs {
        config.collection_interval_secs = interval;
    }
    if let Some(days) = settings.retention_days {
        config.retention_days = days;
    }
    if let Some(aot) = settings.always_on_top {
        config.window.always_on_top = aot;
    }
    if let Some(ct) = settings.click_through {
        config.window.click_through = ct;
    }
    if let Some(pct) = settings.notification_warning_pct {
        config.notification_warning_pct = pct;
    }
    if let Some(pct) = settings.notification_critical_pct {
        config.notification_critical_pct = pct;
    }

    // Tell every window about a locale change so the widget and the dashboard
    // stay in the same language
    if let Some(ref locale) = settings.locale {
        use tauri::Emitter;
        if let Err(e) = app.emit("locale-changed", locale.clone()) {
            log::warn!("Failed to broadcast locale change: {}", e);
        }
        #[cfg(not(test))]
        crate::tray::apply_locale(&app, locale);
    }

    // Notification thresholds take effect on the running scheduler
    if settings.notification_warning_pct.is_some() || settings.notification_critical_pct.is_some() {
        state.scheduler.set_notification_thresholds(
            config.notification_warning_pct,
            config.notification_critical_pct,
        );
    }

    // A new collection interval takes effect on the running loop right away
    // (it restarts its pending wait), not only after the next app launch.
    if let Some(interval) = settings.collection_interval_secs {
        state.scheduler.set_interval(interval);
    }

    // Apply window settings to the running widget, not just to the file
    #[cfg(not(test))]
    {
        if let Some(aot) = settings.always_on_top {
            crate::window::tauri_ops::set_always_on_top(&app, &state.window_manager, aot)?;
        }
        if let Some(ct) = settings.click_through {
            crate::window::tauri_ops::set_click_through(&app, &state.window_manager, ct)?;
        }
    }

    // Autostart lives in the Windows registry, not in the config file
    if let Some(enabled) = settings.autostart {
        let result = if enabled {
            std::env::current_exe()
                .map_err(|e| format!("Cannot resolve executable path: {}", e))
                .and_then(|exe| crate::tray::register_autostart(&exe).map_err(|e| e.to_string()))
        } else {
            crate::tray::unregister_autostart().map_err(|e| e.to_string())
        };
        result.map_err(|e| format!("Failed to update autostart: {}", e))?;
    }

    // Persist to disk
    config
        .save_to_file(&state.config_path)
        .map_err(|e| format!("Failed to save settings: {}", e))?;

    Ok(())
}

/// Trigger backup to specified path.
#[tauri::command]
pub async fn backup_data(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    if path.is_empty() {
        return Err("Backup path must not be empty".to_string());
    }

    let dest = PathBuf::from(&path);

    state
        .storage
        .backup(&dest)
        .await
        .map_err(|e| format!("Backup failed: {}", e))
}

/// Restore from backup file.
#[tauri::command]
pub async fn restore_data(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    if path.is_empty() {
        return Err("Restore path must not be empty".to_string());
    }

    let src = PathBuf::from(&path);

    if !src.exists() {
        return Err(format!("Backup file does not exist: {}", path));
    }

    state
        .storage
        .restore(&src)
        .await
        .map_err(|e| format!("Restore failed: {}", e))
}

/// Check GitHub API for available updates (compare semver).
#[tauri::command]
pub async fn check_for_updates() -> Result<Option<UpdateInfo>, String> {
    let current_version = env!("CARGO_PKG_VERSION");

    // Fail fast if our own version is not valid semver
    semver::Version::parse(current_version).map_err(|e| {
        format!(
            "Failed to parse current version '{}': {}",
            current_version, e
        )
    })?;

    // Every outbound request goes through the network guard: only
    // api.github.com is reachable, provider APIs are blocked outright.
    const RELEASES_URL: &str = "https://api.github.com/repos/ai-usage-widget/releases/latest";
    let guard = crate::network::NetworkGuard::new();
    if !guard.is_allowed(RELEASES_URL) {
        let audit = guard.audit_blocked_request(RELEASES_URL);
        log::warn!("Update check blocked by network guard: {:?}", audit);
        return Err("Update check blocked by network policy".to_string());
    }

    // Query GitHub releases API
    let client = reqwest::Client::builder()
        .user_agent("ai-usage-widget")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| format!("Failed to check for updates: {}", e))?;

    if !response.status().is_success() {
        // No update info available (e.g., 404, rate limited)
        return Ok(None);
    }

    let release: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse release response: {}", e))?;

    // Extract tag_name (e.g., "v0.2.0")
    let tag = release["tag_name"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches('v');

    // Same comparison the unit tests exercise
    match compare_versions(current_version, tag) {
        Some(latest_version) => {
            let download_url = release["html_url"].as_str().unwrap_or("").to_string();
            let release_notes = release["body"].as_str().map(|s| s.to_string());

            Ok(Some(UpdateInfo {
                current_version: current_version.to_string(),
                latest_version,
                download_url,
                release_notes,
            }))
        }
        None => Ok(None),
    }
}

// ─── Testable Helper Functions ──────────────────────────────────────────────────

/// Compare two version strings and report the newer one.
/// Returns `Some(latest)` when `latest` is newer than `current`.
fn compare_versions(current: &str, latest: &str) -> Option<String> {
    let current_v = semver::Version::parse(current).ok()?;
    let latest_v = semver::Version::parse(latest).ok()?;

    if latest_v > current_v {
        Some(latest_v.to_string())
    } else {
        None
    }
}

// ─── Property-Based Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod prop_tests_version_comparison {
    use super::compare_versions;
    use proptest::prelude::*;

    // **Validates: Requirements 12.5**

    /// Strategy to generate valid semver version components (0-99 range for practical testing)
    fn version_component() -> impl Strategy<Value = u32> {
        0u32..100u32
    }

    /// Strategy to generate a valid semver version string "major.minor.patch"
    fn semver_version() -> impl Strategy<Value = String> {
        (
            version_component(),
            version_component(),
            version_component(),
        )
            .prop_map(|(major, minor, patch)| format!("{}.{}.{}", major, minor, patch))
    }

    proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(50))]

        /// Property 1: Reflexivity - Any version equals itself (not greater, not less).
        #[test]
        fn reflexivity(version in semver_version()) {
            // A version compared against itself should indicate no update available
            let result = compare_versions(&version, &version);
            prop_assert!(result.is_none(), "Version {} compared to itself should return None, got {:?}", version, result);
        }

        /// Property 2: Transitivity - If v1 > v2 and v2 > v3, then v1 > v3.
        #[test]
        fn transitivity(
            a in version_component(),
            b in version_component(),
            c in version_component(),
            base_minor in version_component(),
            base_patch in version_component(),
        ) {
            // Create three ordered versions by using sorted major components
            let mut vals = [a % 50, b % 50, c % 50];
            vals.sort();
            // Ensure they are distinct
            let v3 = format!("{}.{}.{}", vals[0], base_minor, base_patch);
            let v2 = format!("{}.{}.{}", vals[0] + 1, base_minor, base_patch);
            let v1 = format!("{}.{}.{}", vals[0] + 2, base_minor, base_patch);

            // v1 > v2 (latest=v1, current=v2 -> update available)
            let r1 = compare_versions(&v2, &v1);
            prop_assert!(r1.is_some(), "Expected v1({}) > v2({})", v1, v2);

            // v2 > v3 (latest=v2, current=v3 -> update available)
            let r2 = compare_versions(&v3, &v2);
            prop_assert!(r2.is_some(), "Expected v2({}) > v3({})", v2, v3);

            // v1 > v3 (latest=v1, current=v3 -> update available)
            let r3 = compare_versions(&v3, &v1);
            prop_assert!(r3.is_some(), "Expected v1({}) > v3({}) by transitivity", v1, v3);
        }

        /// Property 3: Patch version bump detected - 0.1.0 → 0.1.1 should indicate an update is available.
        #[test]
        fn patch_bump_detected(
            major in version_component(),
            minor in version_component(),
            patch in 0u32..99u32,
        ) {
            let current = format!("{}.{}.{}", major, minor, patch);
            let latest = format!("{}.{}.{}", major, minor, patch + 1);
            let result = compare_versions(&current, &latest);
            prop_assert!(result.is_some(), "Patch bump {} -> {} should indicate update", current, latest);
            prop_assert_eq!(result.unwrap(), latest);
        }

        /// Property 4: Minor version bump detected - 0.1.0 → 0.2.0 should indicate an update is available.
        #[test]
        fn minor_bump_detected(
            major in version_component(),
            minor in 0u32..99u32,
            patch in version_component(),
        ) {
            let current = format!("{}.{}.{}", major, minor, patch);
            let latest = format!("{}.{}.{}", major, minor + 1, 0);
            let result = compare_versions(&current, &latest);
            prop_assert!(result.is_some(), "Minor bump {} -> {} should indicate update", current, latest);
        }

        /// Property 5: Major version bump detected - 0.1.0 → 1.0.0 should indicate an update is available.
        #[test]
        fn major_bump_detected(
            major in 0u32..99u32,
            minor in version_component(),
            patch in version_component(),
        ) {
            let current = format!("{}.{}.{}", major, minor, patch);
            let latest = format!("{}.{}.{}", major + 1, 0, 0);
            let result = compare_versions(&current, &latest);
            prop_assert!(result.is_some(), "Major bump {} -> {} should indicate update", current, latest);
        }

        /// Property 6: Same version no update - When current == latest, no update should be indicated.
        #[test]
        fn same_version_no_update(version in semver_version()) {
            let result = compare_versions(&version, &version);
            prop_assert!(result.is_none(), "Same version {} should not indicate update", version);
        }

        /// Property 7: Pre-release ordering - Pre-release versions (e.g., 1.0.0-alpha < 1.0.0) follow semver rules.
        #[test]
        fn prerelease_ordering(
            major in 1u32..50u32,
            minor in version_component(),
            patch in version_component(),
        ) {
            let prerelease = format!("{}.{}.{}-alpha", major, minor, patch);
            let release = format!("{}.{}.{}", major, minor, patch);

            // Pre-release < release per semver spec
            let result = compare_versions(&prerelease, &release);
            prop_assert!(result.is_some(), "Pre-release {} should be less than release {}", prerelease, release);

            // Release is NOT less than pre-release
            let result_reverse = compare_versions(&release, &prerelease);
            prop_assert!(result_reverse.is_none(), "Release {} should not indicate update to pre-release {}", release, prerelease);
        }
    }
}

#[cfg(test)]
#[path = "commands_freshness_tests.rs"]
mod provider_status_freshness_tests;

#[cfg(test)]
mod provider_status_mapping_tests {
    use super::*;
    use crate::provider::QuotaResets;
    use chrono::{Duration, Utc};

    fn summary(quota_resets: QuotaResets) -> ProviderSummary {
        ProviderSummary {
            provider_id: "codex".to_string(),
            display_name: "Codex Desktop".to_string(),
            is_available: true,
            current_model: None,
            tokens_today: None,
            quota: None,
            context_window: None,
            last_activity: None,
            quota_resets,
            quota_observed: Default::default(),
        }
    }

    #[test]
    fn test_estimated_flag_is_carried_to_the_ui_not_assumed() {
        // Claude's reset times are reconstructed from its usage history, and
        // the widget marks them "~" on the strength of this flag alone. A
        // constant here would silently present a guess as a published fact.
        let resets = QuotaResets {
            fast_hours: Some(Utc::now() + Duration::hours(3)),
            weekly: None,
            estimated: true,
        };
        assert!(provider_status_response(summary(resets)).quota_resets_estimated);

        let published = QuotaResets {
            fast_hours: Some(Utc::now() + Duration::hours(3)),
            weekly: None,
            estimated: false,
        };
        assert!(!provider_status_response(summary(published)).quota_resets_estimated);
    }

    #[test]
    fn test_reset_times_are_serialised_per_window() {
        // The two windows expire independently; crossing them would tell the
        // user a weekly wait is hours away, or the reverse.
        let fast = Utc::now() + Duration::hours(2);
        let weekly = Utc::now() + Duration::days(5);
        let response = provider_status_response(summary(QuotaResets {
            fast_hours: Some(fast),
            weekly: Some(weekly),
            estimated: false,
        }));

        assert_eq!(response.quota_fast_resets_at, Some(fast.to_rfc3339()));
        assert_eq!(response.quota_weekly_resets_at, Some(weekly.to_rfc3339()));
    }
}
