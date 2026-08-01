/**
 * CompactWidget - Main 340×200px compact widget view.
 *
 * Displays token usage summary per provider with auto-refresh every 10 seconds.
 * Uses Windows 11 glass (Acrylic) visual effect via backdrop-filter.
 */

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store";
import {
  formatTokenCountCompact,
  formatRelative,
  formatResetTime,
  formatCountdown,
} from "../lib/format";
import { openDashboard, setWidgetCollapsed, getWidgetCollapsed } from "../lib/ipc";
import { toggleMaximizeWindow } from "../lib/tauri";
import { StatusDot } from "./StatusDot";
import { ProviderMeter } from "./ProviderMeter";
import { LoadingSkeleton } from "./LoadingSkeleton";

/** One metered quota window, ready to render. */
interface QuotaWindow {
  /** i18n key for the window's name. */
  labelKey: string;
  /** Percentage consumed, or null while the provider is not reporting it. */
  usedPct: number | null;
  /** When this window rolls over (RFC 3339), if the provider says. */
  resetsAt: string | null;
  /** True when `resetsAt` was derived from history rather than published. */
  estimated: boolean;
}

/**
 * The quota windows a provider meters, in the order they matter.
 *
 * Both vendors cap a short rolling window and a long one independently, and
 * they run out at different times — a full weekly limit says nothing about
 * whether the next five hours are usable, so they are never merged.
 *
 * Both rows are always listed. Codex stops publishing its five-hour window
 * while the weekly one is the binding limit, and a row that vanishes reads as
 * "this limit is gone" rather than "nothing to report right now".
 */
function quotaWindows(provider: {
  quota_fast_pct: number | null;
  quota_standard_pct: number | null;
  quota_excess_pct: number | null;
  quota_fast_resets_at: string | null;
  quota_weekly_resets_at: string | null;
  quota_resets_estimated: boolean;
}): QuotaWindow[] {
  const estimated = provider.quota_resets_estimated;
  const windows: QuotaWindow[] = [
    {
      labelKey: "quota.fastHours",
      usedPct: provider.quota_fast_pct,
      resetsAt: provider.quota_fast_resets_at,
      estimated,
    },
    {
      labelKey: "quota.standard",
      usedPct: provider.quota_standard_pct,
      resetsAt: provider.quota_weekly_resets_at,
      estimated,
    },
  ];

  // Excess only exists on plans that allow it, so it stays conditional
  if (typeof provider.quota_excess_pct === "number") {
    windows.push({
      labelKey: "quota.excess",
      usedPct: provider.quota_excess_pct,
      resetsAt: null,
      estimated: false,
    });
  }

  return windows;
}

/** Most-consumed window, for the one-line collapsed summary. */
function highestQuota(provider: Parameters<typeof quotaWindows>[0]): number | null {
  const reported = quotaWindows(provider)
    .map((w) => w.usedPct)
    .filter((pct): pct is number => pct !== null);

  return reported.length > 0 ? Math.max(...reported) : null;
}

/** Quota left, from the consumed percentage the providers report. */
function remainingPct(usedPct: number): number {
  return Math.max(0, Math.min(100, 100 - usedPct));
}

export function CompactWidget() {
  const { t } = useTranslation();
  const { usage, providers, isLoading, fetchUsage, fetchProviderStatus } = useAppStore();
  const [collapsed, setCollapsed] = useState(false);

  const toggleCollapsed = () => {
    const next = !collapsed;
    setCollapsed(next);
    void setWidgetCollapsed(next).catch((err) => {
      console.warn("Could not resize the widget:", err);
      setCollapsed(!next);
    });
  };

  // React owns the collapsed flag at runtime, the window owns the height, and
  // only the backend remembers either across launches. On mount the persisted
  // value is the truth and both sides are set from it — which also repairs the
  // case where a webview reload resets React but leaves the window at 40px,
  // stranding an expanded UI in a strip too short to show the way out.
  useEffect(() => {
    getWidgetCollapsed()
      .then(async (persisted) => {
        // The window is told first: adopting the flag before the resize is
        // accepted would leave React describing a shape the window refused.
        await setWidgetCollapsed(persisted);
        setCollapsed(persisted);
      })
      .catch((err) => {
        console.warn("Could not restore the collapsed state:", err);
      });
  }, []);

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
    <div
      className={`widget-glass w-screen h-screen rounded-lg flex flex-col overflow-hidden ${
        collapsed ? "px-3 py-1.5" : "p-3"
      }`}
    >
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
          {/* Collapsed, the header is all there is — carry the numbers up here */}
          {collapsed &&
            providers.map((provider) => {
              const quota = highestQuota(provider);
              if (quota === null) return null;
              const left = remainingPct(quota);
              return (
                <span
                  key={provider.provider_id}
                  data-tauri-drag-region
                  title={provider.display_name}
                  className={`text-[10px] font-mono ${
                    left <= 10 ? "text-red-300" : left <= 25 ? "text-amber-300" : "text-emerald-300"
                  }`}
                >
                  {provider.provider_id === "codex" ? "CX" : "CL"} {left.toFixed(0)}%
                </span>
              );
            })}
          <span data-tauri-drag-region className="text-[10px] text-white/50">
            {formatTokenCountCompact(usage?.total_tokens_today ?? null)}
          </span>
          <button
            type="button"
            title={collapsed ? t("actions.expand") : t("actions.collapse")}
            aria-label={collapsed ? t("actions.expand") : t("actions.collapse")}
            aria-expanded={!collapsed}
            onClick={toggleCollapsed}
            className="w-5 h-5 flex items-center justify-center rounded text-white/60 hover:text-white hover:bg-white/10 transition-colors"
          >
            <svg viewBox="0 0 16 16" className="w-3 h-3" aria-hidden="true">
              <path
                d={collapsed ? "M4 6.5L8 10.5L12 6.5" : "M4 9.5L8 5.5L12 9.5"}
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
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

      {/* Provider list — collapsed, the header is the whole widget */}
      {!collapsed && (
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
              windows={quotaWindows(provider)}
            />
          );
        })}

        {providers.length === 0 && !isLoading && (
          <div className="flex-1 flex items-center justify-center">
            <span className="text-xs text-white/50">{t("status.notAvailable")}</span>
          </div>
        )}
      </div>
      )}

      {/* Footer */}
      {!collapsed && (
      <div className="flex items-center justify-between border-t border-white/10 pt-1">
        <span className="text-[10px] text-white/40">
          {usage?.last_updated
            ? `${t("time.lastUpdated")}: ${formatRelative(usage.last_updated)}`
            : ""}
        </span>
      </div>
      )}
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
  /** Every quota window this provider meters, each with its own reset. */
  windows: QuotaWindow[];
}

function ProviderRow({
  providerId,
  displayName,
  isAvailable,
  totalTokensToday,
  lastActivity,
  windows,
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
      {/* One meter per window: they run out independently, and which one is
          binding is the whole question when the widget says 0%. */}
      {windows.map((window) => {
        const left = window.usedPct === null ? null : remainingPct(window.usedPct);
        const countdown = formatCountdown(window.resetsAt, {
          approximate: window.estimated,
        });
        // The wall-clock time stays reachable on hover, alongside the reason
        // the number is soft when it was reconstructed rather than published.
        const detail = window.resetsAt
          ? [formatResetTime(window.resetsAt), window.estimated ? t("quota.estimated") : null]
              .filter(Boolean)
              .join(" · ")
          : undefined;

        return (
          <div key={window.labelKey} className="flex flex-col">
            <ProviderMeter
              label={t(window.labelKey)}
              percentage={left}
              valueText={left === null ? "—" : `${left.toFixed(0)}%`}
              danger="low"
            />
            {countdown && (
              <span
                title={detail}
                className={`text-[9px] pl-4 truncate ${
                  left !== null && left <= 0 ? "text-red-300/80" : "text-white/40"
                }`}
              >
                {t("quota.resetsAt")}: {countdown}
              </span>
            )}
          </div>
        );
      })}
      {lastActivity && (
        <span className="text-[9px] text-white/40 pl-4 truncate">
          {formatRelative(lastActivity)}
        </span>
      )}
    </div>
  );
}
