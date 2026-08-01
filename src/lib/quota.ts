/**
 * Turning a provider status into the quota rows both views render.
 *
 * The widget and the dashboard show the same readings in different shapes. The
 * shaping lives here so they cannot disagree about which reset belongs to which
 * window, or about which window is the binding one.
 */

import type { ProviderStatus } from "./ipc";
import type { Freshness } from "./freshness";

/** One metered quota window, ready to render. */
export interface QuotaWindow {
  /** i18n key for the window's full name. */
  labelKey: string;
  /** i18n key for the short code used where space is tight. */
  codeKey: string;
  /** Percentage consumed, or null while the provider is not reporting it. */
  usedPct: number | null;
  /** When this window rolls over (RFC 3339), if the provider says. */
  resetsAt: string | null;
  /** True when `resetsAt` was derived from history rather than published. */
  estimated: boolean;
  /** How current the percentage is, as classified by the backend. */
  freshness: Freshness;
  /** Age of the reading in seconds, when it can be determined. */
  ageSecs: number | null;
  /** When the source record was written (RFC 3339), for tooltips. */
  observedAt: string | null;
}

/** The subset of a provider status these helpers need. */
export type QuotaSource = Pick<
  ProviderStatus,
  | "quota_fast_pct"
  | "quota_standard_pct"
  | "quota_excess_pct"
  | "quota_fast_resets_at"
  | "quota_weekly_resets_at"
  | "quota_resets_estimated"
  | "quota_fast_observed_at"
  | "quota_weekly_observed_at"
  | "quota_fast_freshness"
  | "quota_weekly_freshness"
  | "quota_fast_age_secs"
  | "quota_weekly_age_secs"
>;

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
export function quotaWindows(provider: QuotaSource): QuotaWindow[] {
  const estimated = provider.quota_resets_estimated;

  const windows: QuotaWindow[] = [
    {
      labelKey: "quota.fastHours",
      codeKey: "quota.fastCode",
      usedPct: provider.quota_fast_pct,
      resetsAt: provider.quota_fast_resets_at,
      estimated,
      freshness: provider.quota_fast_freshness,
      ageSecs: provider.quota_fast_age_secs,
      observedAt: provider.quota_fast_observed_at,
    },
    {
      labelKey: "quota.standard",
      codeKey: "quota.weeklyCode",
      usedPct: provider.quota_standard_pct,
      resetsAt: provider.quota_weekly_resets_at,
      estimated,
      freshness: provider.quota_weekly_freshness,
      ageSecs: provider.quota_weekly_age_secs,
      observedAt: provider.quota_weekly_observed_at,
    },
  ];

  // Excess only exists on plans that allow it, so it stays conditional. It
  // rides on the weekly reading that reported it, so it inherits its timing.
  if (typeof provider.quota_excess_pct === "number") {
    windows.push({
      labelKey: "quota.excess",
      codeKey: "quota.excessCode",
      usedPct: provider.quota_excess_pct,
      resetsAt: null,
      estimated: false,
      freshness: provider.quota_weekly_freshness,
      ageSecs: provider.quota_weekly_age_secs,
      observedAt: provider.quota_weekly_observed_at,
    });
  }

  return windows;
}

/** Quota left, from the consumed percentage the providers report. */
export function remainingPct(usedPct: number): number {
  return Math.max(0, Math.min(100, 100 - usedPct));
}

/**
 * The window closest to running out — the one actually constraining the user.
 *
 * Returned whole rather than as a bare number: "0%" alone cannot say whether
 * the wait is hours or days, which is the only thing the reader wants to know
 * when the widget is collapsed to one line.
 *
 * Expired windows are skipped. Their percentage describes a window that has
 * already rolled over, so treating one as binding would pin the summary to a
 * number known to be wrong.
 */
export function bindingWindow(provider: QuotaSource): QuotaWindow | null {
  const usable = quotaWindows(provider).filter(
    (w) => w.usedPct !== null && w.freshness !== "expired",
  );

  return usable.reduce<QuotaWindow | null>((worst, candidate) => {
    if (worst === null) return candidate;
    return (candidate.usedPct ?? 0) > (worst.usedPct ?? 0) ? candidate : worst;
  }, null);
}
