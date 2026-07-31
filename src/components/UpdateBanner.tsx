/**
 * UpdateBanner - Notification banner shown when a new version is available.
 *
 * Checks for updates on mount via the checkForUpdates() IPC command.
 * Displays current version, latest version, and a download link that opens
 * in the user's default browser. The banner is dismissible and renders
 * nothing when no update is available.
 */

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { checkForUpdates, UpdateInfo } from "../lib/ipc";

export function UpdateBanner() {
  const { t } = useTranslation();
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [dismissed, setDismissed] = useState(false);

  // Check for updates on component mount (startup)
  useEffect(() => {
    checkForUpdates()
      .then((info) => {
        if (info) {
          setUpdateInfo(info);
        }
      })
      .catch((err) => {
        // Silently ignore update check failures — not critical
        console.warn("Update check failed:", err);
      });
  }, []);

  // Render nothing if no update available or dismissed
  if (!updateInfo || dismissed) {
    return null;
  }

  const handleDownload = async () => {
    try {
      await openUrl(updateInfo.download_url);
    } catch (err) {
      console.warn("Failed to open download URL:", err);
    }
  };

  const handleDismiss = () => {
    setDismissed(true);
  };

  return (
    <div className="w-full bg-blue-500/20 border border-blue-400/30 rounded-md px-3 py-2 flex flex-col gap-1">
      {/* Header row: title + dismiss button */}
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-semibold text-blue-300">
          {t("update.available")}
        </span>
        <button
          type="button"
          onClick={handleDismiss}
          className="text-white/50 hover:text-white/80 text-xs leading-none p-0.5"
          aria-label={t("update.dismiss")}
        >
          ✕
        </button>
      </div>

      {/* Version info */}
      <div className="flex items-center gap-2 text-[10px] text-white/70">
        <span>
          {t("update.currentVersion")}: {updateInfo.current_version}
        </span>
        <span>→</span>
        <span>
          {t("update.latestVersion")}: {updateInfo.latest_version}
        </span>
      </div>

      {/* Release notes (optional) */}
      {updateInfo.release_notes && (
        <p className="text-[10px] text-white/50 line-clamp-2">
          {t("update.releaseNotes")}: {updateInfo.release_notes}
        </p>
      )}

      {/* Download button */}
      <button
        type="button"
        onClick={handleDownload}
        className="self-start mt-1 px-2 py-0.5 text-[10px] font-medium text-white bg-blue-500/40 hover:bg-blue-500/60 rounded transition-colors"
      >
        {t("update.download")}
      </button>
    </div>
  );
}
