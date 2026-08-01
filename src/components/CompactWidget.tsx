/**
 * CompactWidget - Main 340×200px compact widget view.
 *
 * Displays token usage summary per provider with auto-refresh every 10 seconds.
 * Uses Windows 11 glass (Acrylic) visual effect via backdrop-filter.
 */

import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store";
import {
  formatTokenCountCompact,
  formatRelative,
  formatResetTime,
} from "../lib/format";
import { openDashboard } from "../lib/ipc";
import { toggleMaximizeWindow } from "../lib/tauri";
import { StatusDot } from "./StatusDot";
import { ProviderMeter } from "./ProviderMeter";
import { LoadingSkeleton } from "./LoadingSkeleton";

/** Highest quota dimension a provider reports, or null if it publishes none. */
function highestQuota(provider: {
  quota_fast_pct: number | null;
  quota_standard_pct: number | null;
  quota_excess_pct: number | null;
}): number | null {
  const values = [
    provider.quota_fast_pct,
    provider.quota_standard_pct,
    provider.quota_excess_pct,
  ].filter((v): v is number => typeof v === "number");

  return values.length > 0 ? Math.max(...values) : null;
}

/** Quota left, from the consumed percentage the providers report. */
function remainingPct(usedPct: number): number {
  return Math.max(0, Math.min(100, 100 - usedPct));
}

export function CompactWidget() {
  const { t } = useTranslation();
  const { usage, providers, isLoading, fetchUsage, fetchProviderStatus } = useAppStore();

  // Fetch on mount and every 10 seconds
  useEffect(() => {
    fetchUsage();
    fetchProviderStatus();

    const interval = setInterval(() => {
      fetchUsage();
      fetchProviderStatus();
    }, 10_000);

    return () => clearInterval(interval);
  }, [fetchUsage, fetchProviderStatus]);

  if (isLoading && !usage) {
    return (
      <div className="widget-glass w-screen h-screen rounded-lg p-3 flex flex-col">
        <LoadingSkeleton />
      </div>
    );
  }

  return (
    <div className="widget-glass w-screen h-screen rounded-lg p-3 flex flex-col overflow-hidden">
      {/* Content stays readable when the window is enlarged or maximized */}
      <div className="flex flex-col gap-2 flex-1 min-h-0 w-full max-w-md mx-auto">
      {/* Header doubles as the drag handle: the window has no title bar */}
      <div
        data-tauri-drag-region
        onDoubleClick={() => void toggleMaximizeWindow()}
        className="flex items-center justify-between gap-2 min-w-0 cursor-move select-none"
      >
        <h1
          data-tauri-drag-region
          className="text-xs font-semibold text-white/90 truncate"
        >
          {t("widget.title")}
        </h1>
        <div className="flex items-center gap-2 shrink-0">
          <span data-tauri-drag-region className="text-[10px] text-white/50">
            {formatTokenCountCompact(usage?.total_tokens_today ?? null)}
          </span>
          <button
            type="button"
            title={t("window.maximize")}
            aria-label={t("window.maximize")}
            onClick={() => void toggleMaximizeWindow()}
            className="w-5 h-5 flex items-center justify-center rounded text-white/60 hover:text-white hover:bg-white/10 transition-colors"
          >
            <svg viewBox="0 0 16 16" className="w-3 h-3" aria-hidden="true">
              <rect
                x="2.5"
                y="2.5"
                width="11"
                height="11"
                rx="1.5"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.6"
              />
            </svg>
          </button>
          <button
            type="button"
            title={t("tray.dashboard")}
            aria-label={t("tray.dashboard")}
            onClick={() => void openDashboard("usage")}
            className="w-5 h-5 flex items-center justify-center rounded text-white/60 hover:text-white hover:bg-white/10 transition-colors"
          >
            {/* Inline SVG — Segoe UI has no glyph for the expand arrows */}
            <svg viewBox="0 0 16 16" className="w-3 h-3" aria-hidden="true">
              <path
                d="M6 2H2v4M10 14h4v-4M14 6V2h-4M2 10v4h4"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
        </div>
      </div>

      {/* Provider list */}
      <div className="flex-1 flex flex-col gap-2 overflow-y-auto overflow-x-hidden">
        {providers.map((provider) => {
          const providerUsage = usage?.providers.find(
            (p) => p.provider_id === provider.provider_id,
          );

          return (
            <ProviderRow
              key={provider.provider_id}
              providerId={provider.provider_id}
              displayName={provider.display_name}
              isAvailable={provider.is_available}
              totalTokensToday={providerUsage?.total_tokens_today ?? null}
              lastActivity={providerUsage?.last_activity ?? null}
              quotaPct={highestQuota(provider)}
              quotaResetsAt={provider.quota_resets_at}
            />
          );
        })}

        {providers.length === 0 && !isLoading && (
          <div className="flex-1 flex items-center justify-center">
            <span className="text-xs text-white/50">{t("status.notAvailable")}</span>
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="flex items-center justify-between border-t border-white/10 pt-1">
        <span className="text-[10px] text-white/40">
          {usage?.last_updated
            ? `${t("time.lastUpdated")}: ${formatRelative(usage.last_updated)}`
            : ""}
        </span>
      </div>
      </div>
    </div>
  );
}

// ─── Provider Row Subcomponent ──────────────────────────────────────────────────

interface ProviderRowProps {
  providerId: string;
  displayName: string;
  isAvailable: boolean;
  totalTokensToday: number | null;
  lastActivity: string | null;
  /** Highest quota dimension the provider reports, in percent. */
  quotaPct: number | null;
  /** When that quota window resets (RFC 3339), if published. */
  quotaResetsAt: string | null;
}

function ProviderRow({
  providerId,
  displayName,
  isAvailable,
  totalTokensToday,
  lastActivity,
  quotaPct,
  quotaResetsAt,
}: ProviderRowProps) {
  const { t } = useTranslation();

  // Determine a display name using i18n keys or fallback to provided name
  const name =
    providerId === "codex"
      ? t("provider.codex")
      : providerId === "claude"
        ? t("provider.claude")
        : displayName;

  if (!isAvailable) {
    return (
      <div className="flex items-center gap-2 py-1">
        <StatusDot available={false} />
        <span className="text-[11px] text-white/70 flex-1">{name}</span>
        <span className="text-[10px] text-white/40">{t("status.notAvailable")}</span>
      </div>
    );
  }

  const tokenText = formatTokenCountCompact(totalTokensToday);

  return (
    <div className="flex flex-col gap-1 py-1 min-w-0">
      <div className="flex items-center gap-2 min-w-0">
        <StatusDot available={true} />
        <span className="text-[11px] text-white/90 flex-1 truncate">{name}</span>
        <span className="text-[10px] text-white/70 shrink-0">{tokenText}</span>
      </div>
      {/* The meter only carries information when the provider reports a quota;
          for token counts the number above already says everything. */}
      {quotaPct !== null && (
        <ProviderMeter
          label={t("quota.remaining")}
          percentage={remainingPct(quotaPct)}
          valueText={`${remainingPct(quotaPct).toFixed(0)}%`}
          danger="low"
        />
      )}
      {quotaPct !== null && remainingPct(quotaPct) <= 0 && quotaResetsAt && (
        <span className="text-[9px] text-red-300/80 pl-4 truncate">
          {t("quota.resetsAt")}: {formatResetTime(quotaResetsAt)}
        </span>
      )}
      {lastActivity && (
        <span className="text-[9px] text-white/40 pl-4 truncate">
          {formatRelative(lastActivity)}
        </span>
      )}
    </div>
  );
}
