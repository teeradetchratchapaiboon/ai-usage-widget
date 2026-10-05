/**
 * Settings - Settings panel UI for the AI Usage Widget.
 *
 * Provides controls for collection interval, language toggle, notification
 * thresholds, autostart, always-on-top, click-through, backup/restore,
 * and data directory display.
 *
 * Validates: Requirements 8.2, 9.2, 10.1, 10.2
 */

import { useEffect, useId, useRef, useState } from "react";
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
import { formatError } from "../lib/commandError";

export function Settings() {
  const { t, i18n } = useTranslation();
  const locale = useAppStore((s) => s.locale);
  const setLocale = useAppStore((s) => s.setLocale);

  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [backupPath, setBackupPath] = useState("");
  const [restorePath, setRestorePath] = useState("");
  // Raw rejection value, formatted at render time so a language switch
  // re-translates an error already on screen.
  const [error, setError] = useState<unknown>(null);
  const [isSaving, setIsSaving] = useState(false);
  const [isCollecting, setIsCollecting] = useState(false);
  const [collectResult, setCollectResult] = useState<string | null>(null);

  // Ids tie each label to its control; declared before the early return so
  // the hook order never changes.
  const intervalId = useId();
  const warningLabelId = useId();
  const criticalLabelId = useId();
  const autostartId = useId();
  const alwaysOnTopId = useId();
  const clickThroughId = useId();
  const backupId = useId();
  const restoreId = useId();

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
      setError(e);
    }
  }

  async function handleCollectNow() {
    setIsCollecting(true);
    setCollectResult(null);
    try {
      const result = await triggerCollection();
      setCollectResult(
        t("settings.collectResult", {
          events: result.events_collected,
          providers: result.providers_collected,
        }),
      );
      setError(result.errors.length > 0 ? result.errors.join("; ") : null);
    } catch (e) {
      setError(e);
    } finally {
      setIsCollecting(false);
    }
  }

  /** Persist a patch; resolves to whether the backend accepted it. */
  async function handleUpdate(patch: Partial<AppSettings>): Promise<boolean> {
    if (!settings) return false;
    setIsSaving(true);
    try {
      await updateSettings(patch);
      setSettings((prev) => prev && { ...prev, ...patch });
      setError(null);
      return true;
    } catch (e) {
      setError(e);
      return false;
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
      setError(e);
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
      setError(e);
    }
  }

  const errorText = error == null ? null : formatError(error, t);

  if (!settings) {
    return (
      <div className="p-4 text-white/70 text-sm">
        {errorText ? (
          <span className="text-red-400">{errorText}</span>
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
      {errorText && (
        <div className="bg-red-500/20 border border-red-400/30 rounded px-3 py-2 text-xs text-red-300">
          {errorText}
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
      <SettingRow label={t("settings.collectionInterval")} controlId={intervalId}>
        <RangeSetting
          id={intervalId}
          min={10}
          max={3600}
          step={10}
          value={settings.collection_interval_secs}
          accentClass="accent-blue-400"
          valueClass="w-12"
          format={(v) => `${v}s`}
          onCommit={(v) => handleUpdate({ collection_interval_secs: v })}
        />
      </SettingRow>

      {/* Language */}
      <SettingRow label={t("settings.language")}>
        <div className="flex gap-2">
          <button
            type="button"
            aria-pressed={locale === "th"}
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
            type="button"
            aria-pressed={locale === "en"}
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
            <span id={warningLabelId} className="text-[10px] text-yellow-300 w-12">
              ⚠ {t("settings.warningThreshold")}
            </span>
            <RangeSetting
              labelledBy={warningLabelId}
              min={1}
              max={100}
              value={settings.notification_warning_pct}
              accentClass="accent-yellow-400"
              valueClass="w-10"
              format={(v) => `${v}%`}
              onCommit={(v) => handleUpdate({ notification_warning_pct: v })}
            />
          </div>
          <div className="flex items-center gap-2">
            <span id={criticalLabelId} className="text-[10px] text-red-300 w-12">
              🚨 {t("settings.criticalThreshold")}
            </span>
            <RangeSetting
              labelledBy={criticalLabelId}
              min={1}
              max={100}
              value={settings.notification_critical_pct}
              accentClass="accent-red-400"
              valueClass="w-10"
              format={(v) => `${v}%`}
              onCommit={(v) => handleUpdate({ notification_critical_pct: v })}
            />
          </div>
        </div>
      </SettingRow>

      {/* Autostart */}
      <SettingRow label={t("settings.autostart")} controlId={autostartId}>
        <ToggleSwitch
          id={autostartId}
          checked={settings.autostart}
          onChange={(v) => handleUpdate({ autostart: v })}
          disabled={isSaving}
        />
      </SettingRow>

      {/* Always on Top */}
      <SettingRow label={t("settings.alwaysOnTop")} controlId={alwaysOnTopId}>
        <ToggleSwitch
          id={alwaysOnTopId}
          checked={settings.always_on_top}
          onChange={(v) => handleUpdate({ always_on_top: v })}
          disabled={isSaving}
        />
      </SettingRow>

      {/* Click-through */}
      <SettingRow label={t("settings.clickThrough")} controlId={clickThroughId}>
        <ToggleSwitch
          id={clickThroughId}
          checked={settings.click_through}
          onChange={(v) => handleUpdate({ click_through: v })}
          disabled={isSaving}
        />
      </SettingRow>

      {/* Backup */}
      <SettingRow label={t("settings.backup")} controlId={backupId}>
        <div className="flex gap-2">
          <input
            id={backupId}
            type="text"
            value={backupPath}
            onChange={(e) => setBackupPath(e.target.value)}
            placeholder={t("settings.backupPathPlaceholder")}
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
      <SettingRow label={t("settings.restore")} controlId={restoreId}>
        <div className="flex gap-2">
          <input
            id={restoreId}
            type="text"
            value={restorePath}
            onChange={(e) => setRestorePath(e.target.value)}
            placeholder={t("settings.backupPathPlaceholder")}
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
  /** Id of the row's single control; multi-control rows leave it unset. */
  controlId?: string;
  children: React.ReactNode;
}

function SettingRow({ label, controlId, children }: SettingRowProps) {
  const labelId = useId();
  const labelClass = "text-xs font-medium text-white/70";

  if (controlId) {
    return (
      <div className="flex flex-col gap-1">
        <label htmlFor={controlId} className={labelClass}>
          {label}
        </label>
        {children}
      </div>
    );
  }

  // Several controls (or none) share this label, so it names the group
  return (
    <div className="flex flex-col gap-1" role="group" aria-labelledby={labelId}>
      <span id={labelId} className={labelClass}>
        {label}
      </span>
      {children}
    </div>
  );
}

interface RangeSettingProps {
  value: number;
  min: number;
  max: number;
  step?: number;
  id?: string;
  labelledBy?: string;
  accentClass: string;
  valueClass: string;
  format: (value: number) => string;
  /** Persists the value; resolves to false when it was rejected. */
  onCommit: (value: number) => Promise<boolean>;
}

/**
 * A slider that only saves when the user lets go.
 *
 * Dragging updates a local draft; the value is committed on pointer release,
 * key release, or blur. Saving on every step sent one IPC round-trip (and a
 * config write) per pixel of drag.
 */
function RangeSetting({
  value,
  min,
  max,
  step,
  id,
  labelledBy,
  accentClass,
  valueClass,
  format,
  onCommit,
}: RangeSettingProps) {
  const [draft, setDraft] = useState(value);
  // pointerup is followed by blur when focus moves on; one commit is enough
  const inFlight = useRef<number | null>(null);

  useEffect(() => {
    setDraft(value);
  }, [value]);

  async function commit() {
    if (draft === value || inFlight.current === draft) return;
    inFlight.current = draft;
    const accepted = await onCommit(draft);
    inFlight.current = null;
    if (!accepted) setDraft(value);
  }

  return (
    <div className="flex flex-1 items-center gap-2">
      <input
        id={id}
        aria-labelledby={labelledBy}
        type="range"
        min={min}
        max={max}
        step={step}
        value={draft}
        onChange={(e) => setDraft(Number(e.target.value))}
        onPointerUp={() => void commit()}
        onKeyUp={() => void commit()}
        onBlur={() => void commit()}
        className={`flex-1 ${accentClass}`}
      />
      <span className={`text-xs text-white/60 text-right ${valueClass}`}>
        {format(draft)}
      </span>
    </div>
  );
}

interface ToggleSwitchProps {
  id?: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}

function ToggleSwitch({ id, checked, onChange, disabled }: ToggleSwitchProps) {
  return (
    <button
      id={id}
      type="button"
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
