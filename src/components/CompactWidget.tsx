/**
 * CompactWidget - the always-on-top compact widget (360x340 by default).
 *
 * Shows each provider's quota windows with their freshness, refreshing every
 * 10 seconds while it is on screen.
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
import {
  openDashboard,
  setWidgetCollapsed,
  getWidgetCollapsed,
  hideWidget,
  WIDGET_VISIBILITY_EVENT,
} from "../lib/ipc";
import { bindingWindow, quotaWindows, remainingPct, type QuotaWindow } from "../lib/quota";
import { presentFreshness, formatUpdatedAgo } from "../lib/freshness";
import { onAppEvent } from "../lib/tauri";
import { StatusDot } from "./StatusDot";
import { ProviderMeter } from "./ProviderMeter";
import { LoadingSkeleton } from "./LoadingSkeleton";

/** Two-letter provider code for the collapsed one-line summary. */
function providerCode(providerId: string): string {
  return providerId === "codex" ? "CX" : "CL";
}

export function CompactWidget() {
  const { t } = useTranslation();
  const { usage, providers, usageLoading, fetchUsage, fetchProviderStatus } = useAppStore();
  const [collapsed, setCollapsed] = useState(false);
  // Assumed on screen until told otherwise: the widget is visible at startup,
  // and a missed event must never leave it silently not updating.
  const [onScreen, setOnScreen] = useState(true);

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

  // Hiding to the tray leaves the webview running, and WebView2 keeps
  // reporting `document.visibilityState === "visible"` for a hidden native
  // window, so the backend says so explicitly instead.
  useEffect(() => onAppEvent<boolean>(WIDGET_VISIBILITY_EVENT, setOnScreen), []);

  // Fetch on mount and every 10 seconds — but only while on screen. Polling
  // into a window sitting in the tray is work nobody can see the result of.
  useEffect(() => {
    if (!onScreen) return;

    // Immediately on becoming visible again: whatever was on screen when it
    // was hidden is by then as old as the time it spent hidden.
    fetchUsage();
    fetchProviderStatus();

    const interval = setInterval(() => {
      fetchUsage();
      fetchProviderStatus();
    }, 10_000);

    return () => clearInterval(interval);
  }, [onScreen, fetchUsage, fetchProviderStatus]);

  if (usageLoading && !usage) {
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
      {/* Content stays readable if the window is resized by dragging */}
      <div className="flex flex-col gap-2 flex-1 min-h-0 w-full max-w-md mx-auto">
      {/* Header doubles as the drag handle: the window has no title bar.
          Double-click used to maximize, which is how the widget ended up
          stretched across the display. */}
      <div
        data-tauri-drag-region
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
              const binding = bindingWindow(provider);
              if (binding === null || binding.usedPct === null) return null;

              const left = remainingPct(binding.usedPct);
              const look = presentFreshness(binding.freshness, binding.ageSecs);

              // Which window is binding is half the answer: 0% on the
              // five-hour limit is a coffee break, 0% on the weekly one is
              // the rest of the week.
              const label = `${providerCode(provider.provider_id)} ${t(binding.codeKey)}`;
              const tooltip = [
                provider.display_name,
                t(binding.labelKey),
                // Via the shared helper rather than composed here: this line
                // used to build the phrase itself and carried the same
                // "Updated just now ago" bug independently.
                look.caption ?? formatUpdatedAgo(binding.ageSecs),
              ]
                .filter(Boolean)
                .join(" · ");

              return (
                <span
                  key={provider.provider_id}
                  data-tauri-drag-region
                  title={tooltip}
                  className={`text-[10px] font-mono ${look.valueClass} ${
                    look.showsValueAsCurrent
                      ? left <= 10
                        ? "text-red-300"
                        : left <= 25
                          ? "text-amber-300"
                          : "text-emerald-300"
                      : "text-white/60"
                  }`}
                >
                  {label} {left.toFixed(0)}%
                  {/* A dot is all the room there is at 40px, but it is enough
                      to stop a stale number reading as a live one */}
                  {!look.showsValueAsCurrent && (
                    <span className="text-amber-300" aria-label={look.badge}>
                      {" "}
                      •
                    </span>
                  )}
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
            title={t("actions.hideToTrayHint")}
            aria-label={t("actions.hideToTray")}
            onClick={() => void hideWidget()}
            className="w-5 h-5 flex items-center justify-center rounded text-white/60 hover:text-white hover:bg-white/10 transition-colors"
          >
            {/* Arrow into a tray: hidden, not closed — collection continues
                and the tray menu brings it back. */}
            <svg viewBox="0 0 16 16" className="w-3 h-3" aria-hidden="true">
              <path
                d="M8 2V9M8 9L5.5 6.5M8 9L10.5 6.5M2.5 10.5V12a1.5 1.5 0 0 0 1.5 1.5h8a1.5 1.5 0 0 0 1.5-1.5v-1.5"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
                strokeLinejoin="round"
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

        {providers.length === 0 && !usageLoading && (
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
        const look = presentFreshness(window.freshness, window.ageSecs);
        // An expired window's percentage describes a window that has already
        // rolled over, so the figure is known to be wrong, not merely old.
        const left =
          window.usedPct === null || look.suppressesValue
            ? null
            : remainingPct(window.usedPct);
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
        const observedTooltip = window.observedAt
          ? `${t("freshness.observedAt")}: ${formatResetTime(window.observedAt)}`
          : look.badge;

        return (
          <div key={window.labelKey} className="flex flex-col">
            <ProviderMeter
              label={t(window.labelKey)}
              percentage={left}
              valueText={left === null ? "—" : `${left.toFixed(0)}%`}
              danger="low"
              valueClassName={look.valueClass}
              muted={!look.showsValueAsCurrent}
            />
            {/* Age before reset: how much the number can be trusted comes
                first, since it decides whether the countdown means anything. */}
            {look.caption && (
              <span
                title={observedTooltip}
                className={`text-[9px] pl-4 truncate ${look.captionClass}`}
              >
                {look.caption}
              </span>
            )}
            {countdown && !look.suppressesValue && (
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
