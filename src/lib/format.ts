/**
 * Time and number formatting utilities.
 *
 * Uses i18next for locale-aware translations and Intl.DateTimeFormat
 * for timezone conversion (Asia/Bangkok).
 */

import i18n from "../i18n";

// ─── Relative Time Formatting ───────────────────────────────────────────────────

/**
 * Format an ISO 8601 timestamp as relative time (e.g., "2 minutes ago" / "2 นาทีที่แล้ว").
 *
 * Computes the difference between now and the given timestamp, then selects
 * the appropriate unit (seconds, minutes, hours, days).
 */
export function formatRelative(isoTimestamp: string): string {
  const then = new Date(isoTimestamp).getTime();
  const now = Date.now();
  const diffMs = now - then;

  // If the timestamp is in the future or invalid, fall back to "0 seconds ago"
  const diffSec = Math.max(0, Math.floor(diffMs / 1000));

  let value: number;
  let unit: string;

  if (diffSec < 60) {
    value = diffSec;
    unit = i18n.t("time.seconds");
  } else if (diffSec < 3600) {
    value = Math.floor(diffSec / 60);
    unit = i18n.t("time.minutes");
  } else if (diffSec < 86400) {
    value = Math.floor(diffSec / 3600);
    unit = i18n.t("time.hours");
  } else {
    value = Math.floor(diffSec / 86400);
    unit = i18n.t("time.days");
  }

  return `${value} ${unit} ${i18n.t("time.ago")}`;
}

// ─── Token Count Formatting ─────────────────────────────────────────────────────

/**
 * Format a token count with thousands separators (e.g., "1,234,567").
 * Returns the localized "Not available" / "ไม่มีข้อมูล" string for null values.
 */
export function formatTokenCount(count: number | null): string {
  if (count === null) {
    return i18n.t("status.notAvailable");
  }
  return count.toLocaleString("en-US");
}

/**
 * Compact token count for tight layouts: 1.2K / 34.5M / 2.1B.
 * Values below 1,000 are printed as-is. Returns "Not available" for null.
 */
export function formatTokenCountCompact(count: number | null): string {
  if (count === null) {
    return i18n.t("status.notAvailable");
  }

  const units: Array<[number, string]> = [
    [1_000_000_000, "B"],
    [1_000_000, "M"],
    [1_000, "K"],
  ];

  for (const [size, suffix] of units) {
    if (Math.abs(count) >= size) {
      const value = count / size;
      const digits = value >= 100 ? 0 : 1;
      return `${value.toFixed(digits)}${suffix}`;
    }
  }

  return count.toLocaleString("en-US");
}

// ─── Percentage Formatting ──────────────────────────────────────────────────────

/**
 * Format a percentage value with one decimal place (e.g., "75.0%").
 * Returns "N/A" for null values.
 */
export function formatPercentage(value: number | null): string {
  if (value === null) {
    return "N/A";
  }
  return `${value.toFixed(1)}%`;
}

// ─── Timezone Conversion ────────────────────────────────────────────────────────

/**
 * Convert an ISO 8601 UTC timestamp to a formatted string in Asia/Bangkok timezone.
 * Returns the localized "Not available" string if the timestamp is null or invalid.
 */
export function formatBangkokTime(isoTimestamp: string | null): string {
  if (isoTimestamp === null) {
    return i18n.t("status.notAvailable");
  }

  const date = new Date(isoTimestamp);
  if (isNaN(date.getTime())) {
    return i18n.t("status.notAvailable");
  }

  const locale = i18n.language === "th" ? "th-TH" : "en-US";

  return new Intl.DateTimeFormat(locale, {
    timeZone: "Asia/Bangkok",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

/**
 * Format when a quota window resets, as local date + time.
 * Returns "Not available" for null/unparseable values.
 */
export function formatResetTime(iso: string | null): string {
  if (!iso) {
    return i18n.t("status.notAvailable");
  }

  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) {
    return i18n.t("status.notAvailable");
  }

  return date.toLocaleString(i18n.language === "th" ? "th-TH" : "en-US", {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/**
 * How long until a quota window rolls over — "in 4 days 2 hrs" / "อีก 4 วัน 2 ชม.".
 *
 * A wall-clock reset time makes the reader subtract dates in their head to
 * answer the only question they actually have: how long is the wait. Two
 * units is all the precision that question needs.
 *
 * Set `approximate` for a time that was reconstructed rather than published;
 * it marks the number itself ("in ~4 days"), since that is what is uncertain.
 *
 * Returns null when the timestamp is missing, unparseable, or already past,
 * so callers drop the line rather than state something untrue. A reset in the
 * past is not "about to happen" — it is a reading too old to say anything
 * about, and Codex hands one over whenever the app has been idle past its
 * five-hour window.
 */
export function formatCountdown(
  iso: string | null,
  options: { approximate?: boolean } = {},
): string | null {
  if (!iso) {
    return null;
  }

  const target = new Date(iso).getTime();
  if (Number.isNaN(target)) {
    return null;
  }

  const diffSec = Math.floor((target - Date.now()) / 1000);
  if (diffSec <= 0) {
    return null;
  }

  // Round the smaller unit rather than truncate: a freshly issued seven-day
  // window floors to "6 days 23 hrs", which reads as a bug rather than as a
  // full week. Rounding can spill into the larger unit, so carry it.
  let days = Math.floor(diffSec / 86400);
  let hours: number;
  let minutes = 0;

  if (days > 0) {
    hours = Math.round((diffSec % 86400) / 3600);
    if (hours === 24) {
      days += 1;
      hours = 0;
    }
  } else {
    hours = Math.floor(diffSec / 3600);
    minutes = Math.round((diffSec % 3600) / 60);
    if (minutes === 60) {
      hours += 1;
      minutes = 0;
    }
  }

  const unit = (key: string, count: number) => `${count} ${i18n.t(key, { count })}`;

  const parts: string[] = [];
  if (days > 0) {
    parts.push(unit("time.dayShort", days));
    if (hours > 0) parts.push(unit("time.hourShort", hours));
  } else if (hours > 0) {
    parts.push(unit("time.hourShort", hours));
    if (minutes > 0) parts.push(unit("time.minuteShort", minutes));
  } else {
    // Under a minute still reads as "1 min" — "0 min" looks like a stuck clock
    parts.push(unit("time.minuteShort", Math.max(1, minutes)));
  }

  const marker = options.approximate ? "~" : "";
  return `${i18n.t("time.in")} ${marker}${parts.join(" ")}`;
}
