use std::path::PathBuf;
use std::sync::Arc;

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::config::AppConfig;
use crate::query_types::{Granularity, TimeRange, UsageRecord, UsageSummary};
use crate::registry::ProviderRegistry;
use crate::scheduler::CollectionScheduler;
use crate::storage::StorageLayer;
use crate::validation::{validate_settings, validate_time_range, PartialSettings};

// ─── Managed Application State ─────────────────────────────────────────────────

/// Shared application state managed by Tauri.
///
/// Holds references to all core subsystems that IPC commands need to access.
pub struct AppState {
    pub storage: Arc<StorageLayer>,
    pub registry: Arc<ProviderRegistry>,
    pub scheduler: Arc<Mutex<CollectionScheduler>>,
    pub config: Arc<Mutex<AppConfig>>,
    /// Path to the config file on disk for persisting changes.
    pub config_path: PathBuf,
    /// Window state, so settings changes reach the live widget.
    pub window_manager: Arc<crate::window::WindowManager>,
}

// ─── Response Types ─────────────────────────────────────────────────────────────

/// Status information for a single registered provider.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatusResponse {
    pub provider_id: String,
    pub display_name: String,
    pub is_available: bool,
    pub last_collection: Option<String>,
    pub events_collected: u64,
    pub errors: Vec<String>,
    /// Quota consumed, in percent, for providers that publish quota data.
    pub quota_fast_pct: Option<f64>,
    pub quota_standard_pct: Option<f64>,
    pub quota_excess_pct: Option<f64>,
    /// Tokens reported by the provider itself for today, when available.
    pub tokens_today: Option<u64>,
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
pub async fn get_current_usage(
    state: tauri::State<'_, AppState>,
) -> Result<UsageSummary, String> {
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
        other => return Err(format!(
            "Invalid granularity '{}': must be one of hourly, daily, weekly, monthly",
            other
        )),
    };

    // Construct and validate time range
    let range = TimeRange {
        start: start_dt,
        end: end_dt,
    };

    validate_time_range(&range)
        .map_err(|e| format!("Time range validation failed: {}", e))?;

    // Query storage
    state
        .storage
        .get_history(&range, gran)
        .await
        .map_err(|e| format!("Failed to get usage history: {}", e))
}

/// Return status of all registered providers.
#[tauri::command]
pub async fn get_provider_status(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProviderStatusResponse>, String> {
    let summaries = state.registry.get_all_summaries();

    let responses: Vec<ProviderStatusResponse> = summaries
        .into_iter()
        .map(|s| ProviderStatusResponse {
            provider_id: s.provider_id,
            display_name: s.display_name,
            is_available: s.is_available,
            last_collection: s.last_activity.map(|dt| dt.to_rfc3339()),
            events_collected: s.tokens_today.unwrap_or(0),
            errors: Vec::new(),
            quota_fast_pct: s.quota.as_ref().and_then(|q| q.fast_hours_pct),
            quota_standard_pct: s.quota.as_ref().and_then(|q| q.standard_pct),
            quota_excess_pct: s.quota.as_ref().and_then(|q| q.excess_pct),
            tokens_today: s.tokens_today,
        })
        .collect();

    Ok(responses)
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

    let mut events_collected: u32 = 0;
    let mut providers_collected: u32 = 0;
    let mut errors: Vec<String> = Vec::new();

    for outcome in outcomes {
        match outcome {
            crate::registry::ProviderCollectionOutcome::Success { provider_id, result } => {
                events_collected += result.events.len() as u32;
                providers_collected += 1;
                // Store events
                if !result.events.is_empty() {
                    // Note: Full dedup/reconcile pipeline would be invoked here.
                    // For now, we report what was collected.
                    log::info!(
                        "Trigger collection: provider '{}' returned {} events",
                        provider_id,
                        result.events.len()
                    );
                }
            }
            crate::registry::ProviderCollectionOutcome::Failed { provider_id, error } => {
                errors.push(format!("Provider '{}': {}", provider_id, error));
            }
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
pub async fn get_settings(
    state: tauri::State<'_, AppState>,
) -> Result<AppSettings, String> {
    let config = state.config.lock().await;

    Ok(AppSettings {
        collection_interval_secs: config.collection_interval_secs,
        locale: config.locale.clone(),
        notification_warning_pct: 75.0,
        notification_critical_pct: 90.0,
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
    validate_settings(&settings)
        .map_err(|e| format!("Settings validation failed: {}", e))?;

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
                .and_then(|exe| {
                    crate::tray::register_autostart(&exe).map_err(|e| e.to_string())
                })
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
pub async fn backup_data(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
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
pub async fn restore_data(
    path: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
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

    // Parse current version
    let current = semver::Version::parse(current_version)
        .map_err(|e| format!("Failed to parse current version '{}': {}", current_version, e))?;

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

    let latest = match semver::Version::parse(tag) {
        Ok(v) => v,
        Err(_) => return Ok(None), // Unparseable version tag, skip
    };

    if latest > current {
        let download_url = release["html_url"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let release_notes = release["body"].as_str().map(|s| s.to_string());

        Ok(Some(UpdateInfo {
            current_version: current.to_string(),
            latest_version: latest.to_string(),
            download_url,
            release_notes,
        }))
    } else {
        Ok(None)
    }
}


// ─── Testable Helper Functions ──────────────────────────────────────────────────

/// Compare two version strings and determine if an update is available.
/// Returns Some(latest) if latest > current, None otherwise.
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

    /// **Validates: Requirements 12.5**

    /// Strategy to generate valid semver version components (0-99 range for practical testing)
    fn version_component() -> impl Strategy<Value = u32> {
        0u32..100u32
    }

    /// Strategy to generate a valid semver version string "major.minor.patch"
    fn semver_version() -> impl Strategy<Value = String> {
        (version_component(), version_component(), version_component())
            .prop_map(|(major, minor, patch)| format!("{}.{}.{}", major, minor, patch))
    }

    /// Strategy to generate a valid semver version with optional pre-release
    fn semver_version_with_prerelease() -> impl Strategy<Value = String> {
        prop_oneof![
            // Plain version
            semver_version(),
            // Version with pre-release identifier
            (version_component(), version_component(), version_component(), prop_oneof![
                Just("alpha".to_string()),
                Just("beta".to_string()),
                Just("rc.1".to_string()),
                Just("rc.2".to_string()),
            ]).prop_map(|(major, minor, patch, pre)| format!("{}.{}.{}-{}", major, minor, patch, pre)),
        ]
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
