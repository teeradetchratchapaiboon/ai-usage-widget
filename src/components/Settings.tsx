/**
 * Settings - Settings panel UI for the AI Usage Widget.
 *
 * Provides controls for collection interval, language toggle, notification
 * thresholds, autostart, always-on-top, click-through, backup/restore,
 * and data directory display.
 *
 * Validates: Requirements 8.2, 9.2, 10.1, 10.2
 */

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store";
import {
  getSettings,
  updateSettings,
  backupData,
  restoreData,
  triggerCollection,
} from "../lib/ipc";
import type { AppSettings } from "../lib/ipc";

export function Settings() {
  const { t, i18n } = useTranslation();
  const { locale, setLocale } = useAppStore();

  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [backupPath, setBackupPath] = useState("");
  const [restorePath, setRestorePath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [isCollecting, setIsCollecting] = useState(false);
  const [collectResult, setCollectResult] = useState<string | null>(null);

  // Load settings on mount
  useEffect(() => {
    loadSettings();
  }, []);

  async function loadSettings() {
    try {
      const current = await getSettings();
      setSettings(current);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleCollectNow() {
    setIsCollecting(true);
    setCollectResult(null);
    try {
      const result = await triggerCollection();
      setCollectResult(
        `+${result.events_collected} (${result.providers_collected} provider)`,
      );
      setError(result.errors.length > 0 ? result.errors.join("; ") : null);
    } catch (e) {
      setError(String(e));
    } finally {
      setIsCollecting(false);
    }
  }

  async function handleUpdate(patch: Partial<AppSettings>) {
    if (!settings) return;
    setIsSaving(true);
    try {
      await updateSettings(patch);
      setSettings({ ...settings, ...patch });
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setIsSaving(false);
    }
  }

  function handleLocaleChange(newLocale: "th" | "en") {
    i18n.changeLanguage(newLocale);
    setLocale(newLocale);
    handleUpdate({ locale: newLocale });
  }

  async function handleBackup() {
    if (!backupPath.trim()) return;
    try {
      await backupData(backupPath.trim());
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function handleRestore() {
    if (!restorePath.trim()) return;
    try {
      await restoreData(restorePath.trim());
      setError(null);
      // Reload settings after restore
      await loadSettings();
    } catch (e) {
      setError(String(e));
    }
  }

  if (!settings) {
    return (
      <div className="p-4 text-white/70 text-sm">
        {error ? (
          <span className="text-red-400">{error}</span>
        ) : (
          <span>{t("status.loading")}</span>
        )}
      </div>
    );
  }

  return (
    <div className="widget-glass rounded-lg p-4 flex flex-col gap-4 text-white/90 text-sm max-w-md w-full">
      <h2 className="text-base font-semibold">{t("tray.settings")}</h2>

      {/* Error display */}
      {error && (
        <div className="bg-red-500/20 border border-red-400/30 rounded px-3 py-2 text-xs text-red-300">
          {error}
        </div>
      )}

      {/* Manual collection — same pipeline as the scheduler */}
      <SettingRow label={t("tray.collectNow")}>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={handleCollectNow}
            disabled={isCollecting}
            className="text-xs px-3 py-1 rounded bg-blue-500/80 hover:bg-blue-500 disabled:opacity-50 transition-colors"
          >
            {t("tray.collectNow")}
          </button>
          {collectResult && (
            <span className="text-xs text-white/60">{collectResult}</span>
          )}
        </div>
      </SettingRow>

      {/* Collection Interval */}
      <SettingRow label={t("settings.collectionInterval")}>
        <div className="flex items-center gap-2">
          <input
            type="range"
            min={10}
            max={3600}
            step={10}
            value={settings.collection_interval_secs}
            onChange={(e) =>
              handleUpdate({ collection_interval_secs: Number(e.target.value) })
            }
            className="flex-1 accent-blue-400"
            disabled={isSaving}
          />
          <span className="text-xs text-white/60 w-12 text-right">
            {settings.collection_interval_secs}s
          </span>
        </div>
      </SettingRow>

      {/* Language */}
      <SettingRow label={t("settings.language")}>
        <div className="flex gap-2">
          <button
            onClick={() => handleLocaleChange("th")}
            className={`px-3 py-1 rounded text-xs transition-colors ${
              locale === "th"
                ? "bg-blue-500/80 text-white"
                : "bg-white/10 text-white/60 hover:bg-white/20"
            }`}
          >
            ไทย
          </button>
          <button
            onClick={() => handleLocaleChange("en")}
            className={`px-3 py-1 rounded text-xs transition-colors ${
              locale === "en"
                ? "bg-blue-500/80 text-white"
                : "bg-white/10 text-white/60 hover:bg-white/20"
            }`}
          >
            English
          </button>
        </div>
      </SettingRow>

      {/* Notification Thresholds */}
      <SettingRow label={t("settings.notifications")}>
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <span className="text-[10px] text-yellow-300 w-12">⚠ 75%</span>
            <input
              type="range"
              min={1}
              max={100}
              value={settings.notification_warning_pct}
              onChange={(e) =>
                handleUpdate({ notification_warning_pct: Number(e.target.value) })
              }
              className="flex-1 accent-yellow-400"
              disabled={isSaving}
            />
            <span className="text-xs text-white/60 w-10 text-right">
              {settings.notification_warning_pct}%
            </span>
          </div>
          <div className="flex items-center gap-2">
            <span className="text-[10px] text-red-300 w-12">🚨 90%</span>
            <input
              type="range"
              min={1}
              max={100}
              value={settings.notification_critical_pct}
              onChange={(e) =>
                handleUpdate({
                  notification_critical_pct: Number(e.target.value),
                })
              }
              className="flex-1 accent-red-400"
              disabled={isSaving}
            />
            <span className="text-xs text-white/60 w-10 text-right">
              {settings.notification_critical_pct}%
            </span>
          </div>
        </div>
      </SettingRow>

      {/* Autostart */}
      <SettingRow label={t("settings.autostart")}>
        <ToggleSwitch
          checked={settings.autostart}
          onChange={(v) => handleUpdate({ autostart: v })}
          disabled={isSaving}
        />
      </SettingRow>

      {/* Always on Top */}
      <SettingRow label={t("settings.alwaysOnTop")}>
        <ToggleSwitch
          checked={settings.always_on_top}
          onChange={(v) => handleUpdate({ always_on_top: v })}
          disabled={isSaving}
        />
      </SettingRow>

      {/* Click-through */}
      <SettingRow label={t("settings.clickThrough")}>
        <ToggleSwitch
          checked={settings.click_through}
          onChange={(v) => handleUpdate({ click_through: v })}
          disabled={isSaving}
        />
      </SettingRow>

      {/* Backup */}
      <SettingRow label={t("settings.backup")}>
        <div className="flex gap-2">
          <input
            type="text"
            value={backupPath}
            onChange={(e) => setBackupPath(e.target.value)}
            placeholder="C:\backup\usage.db"
            className="flex-1 bg-white/10 rounded px-2 py-1 text-xs text-white/80 placeholder:text-white/30 outline-none focus:ring-1 focus:ring-blue-400/50"
          />
          <button
            onClick={handleBackup}
            disabled={!backupPath.trim()}
            className="px-3 py-1 rounded text-xs bg-blue-500/70 hover:bg-blue-500/90 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
          >
            {t("settings.backup")}
          </button>
        </div>
      </SettingRow>

      {/* Restore */}
      <SettingRow label={t("settings.restore")}>
        <div className="flex gap-2">
          <input
            type="text"
            value={restorePath}
            onChange={(e) => setRestorePath(e.target.value)}
            placeholder="C:\backup\usage.db"
            className="flex-1 bg-white/10 rounded px-2 py-1 text-xs text-white/80 placeholder:text-white/30 outline-none focus:ring-1 focus:ring-blue-400/50"
          />
          <button
            onClick={handleRestore}
            disabled={!restorePath.trim()}
            className="px-3 py-1 rounded text-xs bg-amber-500/70 hover:bg-amber-500/90 disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
          >
            {t("settings.restore")}
          </button>
        </div>
      </SettingRow>

      {/* Data Directory */}
      <SettingRow label={t("settings.dataDirectory")}>
        <span className="text-xs text-white/50 break-all">
          {settings.data_dir || "—"}
        </span>
      </SettingRow>
    </div>
  );
}

// ─── Subcomponents ──────────────────────────────────────────────────────────────

interface SettingRowProps {
  label: string;
  children: React.ReactNode;
}

function SettingRow({ label, children }: SettingRowProps) {
  return (
    <div className="flex flex-col gap-1">
      <label className="text-xs font-medium text-white/70">{label}</label>
      {children}
    </div>
  );
}

interface ToggleSwitchProps {
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}

function ToggleSwitch({ checked, onChange, disabled }: ToggleSwitchProps) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      disabled={disabled}
      className={`relative w-10 h-5 rounded-full transition-colors ${
        checked ? "bg-blue-500/80" : "bg-white/20"
      } ${disabled ? "opacity-50 cursor-not-allowed" : "cursor-pointer"}`}
    >
      <span
        className={`absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white transition-transform ${
          checked ? "translate-x-5" : "translate-x-0"
        }`}
      />
    </button>
  );
}
