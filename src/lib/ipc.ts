/**
 * IPC Bridge - Type-safe wrappers around Tauri invoke() calls.
 *
 * Each function maps to a Rust command handler defined in src-tauri/src/commands.rs.
 * Types mirror the Rust serialization output (snake_case field names).
 */

import { invoke } from "@tauri-apps/api/core";

// ─── TypeScript Types (matching Rust response types) ────────────────────────────

/** Per-provider usage summary for the current period */
export interface ProviderUsageSummary {
  provider_id: string;
  input_tokens_today: number | null;
  output_tokens_today: number | null;
  total_tokens_today: number | null;
  input_tokens_this_week: number | null;
  output_tokens_this_week: number | null;
  total_tokens_this_week: number | null;
  last_activity: string | null;
}

/** Summary of current usage across all providers */
export interface UsageSummary {
  providers: ProviderUsageSummary[];
  total_tokens_today: number | null;
  total_tokens_this_week: number | null;
  last_updated: string;
}

/** Aggregated usage record for a time bucket */
export interface UsageRecord {
  timestamp: string;
  provider_id: string;
  model: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  total_tokens: number | null;
  quota_fast_pct: number | null;
  quota_standard_pct: number | null;
}

/** Status information for a single registered provider */
export interface ProviderStatus {
  provider_id: string;
  display_name: string;
  is_available: boolean;
  last_collection: string | null;
  events_collected: number;
  errors: string[];
  /** Quota consumed in percent, for providers that publish quota data */
  quota_fast_pct: number | null;
  quota_standard_pct: number | null;
  quota_excess_pct: number | null;
  /** Tokens the provider itself reports for today */
  tokens_today: number | null;
  /** When the five-hour window resets (RFC 3339), if known */
  quota_fast_resets_at: string | null;
  /** When the weekly window resets (RFC 3339), if known */
  quota_weekly_resets_at: string | null;
  /** True when the reset times were derived from history, not published */
  quota_resets_estimated: boolean;
}

/** Result of triggering an immediate collection cycle */
export interface CollectionResponse {
  events_collected: number;
  providers_collected: number;
  errors: string[];
}

/** Application settings exposed to the frontend */
export interface AppSettings {
  collection_interval_secs: number;
  locale: string;
  notification_warning_pct: number;
  notification_critical_pct: number;
  autostart: boolean;
  always_on_top: boolean;
  click_through: boolean;
  /** Directory holding the database and config file */
  data_dir: string;
}

/** Information about an available update */
export interface UpdateInfo {
  current_version: string;
  latest_version: string;
  download_url: string;
  release_notes: string | null;
}

// ─── IPC Functions ──────────────────────────────────────────────────────────────

/** Query current usage summary (today/this week totals per provider) */
export async function getCurrentUsage(): Promise<UsageSummary> {
  return invoke<UsageSummary>("get_current_usage");
}

/** Query usage history with time range and granularity */
export async function getUsageHistory(
  start: string,
  end: string,
  granularity: string,
): Promise<UsageRecord[]> {
  return invoke<UsageRecord[]>("get_usage_history", { start, end, granularity });
}

/** Get status of all registered providers */
export async function getProviderStatus(): Promise<ProviderStatus[]> {
  return invoke<ProviderStatus[]>("get_provider_status");
}

/** Trigger an immediate collection cycle */
export async function triggerCollection(): Promise<CollectionResponse> {
  return invoke<CollectionResponse>("trigger_collection");
}

/** Read current application settings */
export async function getSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("get_settings");
}

/** Validate and persist settings changes */
export async function updateSettings(
  settings: Partial<AppSettings>,
): Promise<void> {
  return invoke<void>("update_settings", { settings });
}

/** Trigger backup to specified path */
export async function backupData(path: string): Promise<void> {
  return invoke<void>("backup_data", { path });
}

/** Restore from backup file */
export async function restoreData(path: string): Promise<void> {
  return invoke<void>("restore_data", { path });
}

/** Check GitHub API for available updates */
export async function checkForUpdates(): Promise<UpdateInfo | null> {
  return invoke<UpdateInfo | null>("check_for_updates");
}

/** Open (or focus) the dashboard window on the given tab. */
export async function openDashboard(tab: "usage" | "settings" = "usage"): Promise<void> {
  return invoke("open_dashboard", { tab });
}

/** Close the dashboard and return to widget mode. */
export async function showWidget(): Promise<void> {
  return invoke("show_widget");
}

/** Collapse the widget to its header strip, or restore its height. */
export async function setWidgetCollapsed(collapsed: boolean): Promise<void> {
  return invoke("set_widget_collapsed", { collapsed });
}
