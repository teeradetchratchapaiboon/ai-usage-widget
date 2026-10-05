/**
 * UpdateBanner - Notification banner shown when a new version is available.
 *
 * Checks for updates on mount via the checkForUpdates() IPC command and shows
 * the current and latest version. "Update now" downloads, verifies and
 * installs the signed release in-app (installUpdate()), with progress, and
 * the app restarts afterwards. The "Release page" button opens the release in
 * the default browser and stays available in every phase as the fallback
 * when the in-app update fails. The banner is dismissible (except while an
 * update is downloading or installing) and renders nothing when no update is
 * available.
 */

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { checkForUpdates, installUpdate, UpdateInfo, UpdateProgress } from "../lib/ipc";
import { formatError } from "../lib/commandError";

type Phase = "idle" | "downloading" | "installing" | "error";

export function UpdateBanner() {
  const { t } = useTranslation();
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  const [downloaded, setDownloaded] = useState(0);
  const [total, setTotal] = useState<number | null>(null);
  const [error, setError] = useState<unknown>(null);

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

  const busy = phase === "downloading" || phase === "installing";

  const handleDownload = async () => {
    try {
      await openUrl(updateInfo.download_url);
    } catch (err) {
      console.warn("Failed to open download URL:", err);
    }
  };

  const handleInstall = async () => {
    setPhase("downloading");
    setDownloaded(0);
    setTotal(null);
    setError(null);
    try {
      await installUpdate((e: UpdateProgress) => {
        switch (e.event) {
          case "started":
            setTotal(e.data.content_length && e.data.content_length > 0 ? e.data.content_length : null);
            break;
          case "progress":
            setDownloaded((d) => d + e.data.chunk_length);
            break;
          case "finished":
            setPhase("installing");
            break;
        }
      });
    } catch (err) {
      console.warn("Update install failed:", err);
      setError(err);
      setPhase("error");
    }
  };

  const handleDismiss = () => {
    setDismissed(true);
  };

  const pct = total ? Math.min(100, Math.round((downloaded / total) * 100)) : null;

  let status: string | null = null;
  if (phase === "downloading") {
    status = pct === null ? t("update.downloadingUnknown") : t("update.downloading", { pct });
  } else if (phase === "installing") {
    status = t("update.installing");
  } else if (phase === "error") {
    status = `${t("update.failed")}: ${formatError(error, t)}`;
  }

  return (
    <div className="w-full bg-blue-500/20 border border-blue-400/30 rounded-md px-3 py-2 flex flex-col gap-1">
      {/* Header row: title + dismiss button */}
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-semibold text-blue-300">
          {t("update.available")}
        </span>
        {!busy && (
          <button
            type="button"
            onClick={handleDismiss}
            className="text-white/50 hover:text-white/80 text-xs leading-none p-0.5"
            aria-label={t("update.dismiss")}
          >
            ✕
          </button>
        )}
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

      {/* Download progress, when the size is known */}
      {phase === "downloading" && pct !== null && (
        <div
          role="progressbar"
          aria-label={t("update.progressLabel")}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct}
          className="h-1 w-full bg-white/10 rounded overflow-hidden"
        >
          <div className="h-full bg-blue-400" style={{ width: `${pct}%` }} />
        </div>
      )}

      {/* Status text for screen readers and sighted users alike */}
      <p
        aria-live="polite"
        className={`text-[10px] ${phase === "error" ? "text-red-300" : "text-white/70"}`}
      >
        {status}
      </p>

      {/* Actions: in-app update + release page fallback */}
      <div className="flex gap-2 mt-1">
        <button
          type="button"
          onClick={handleInstall}
          disabled={busy}
          className="px-2 py-0.5 text-[10px] font-medium text-white bg-blue-500/70 hover:bg-blue-500/90 disabled:opacity-50 disabled:cursor-not-allowed rounded transition-colors"
        >
          {t("update.installNow")}
        </button>
        <button
          type="button"
          onClick={handleDownload}
          className="px-2 py-0.5 text-[10px] font-medium text-white bg-blue-500/40 hover:bg-blue-500/60 rounded transition-colors"
        >
          {t("update.download")}
        </button>
      </div>
    </div>
  );
}
