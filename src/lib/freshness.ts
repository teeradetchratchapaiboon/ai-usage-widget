/**
 * How a quota reading's age is presented.
 *
 * The backend classifies freshness against one clock and sends the verdict
 * over IPC; this module decides what that verdict looks like. Keeping the
 * decision here is what stops the widget and the dashboard from disagreeing
 * about whether the same number is trustworthy.
 *
 * Freshness is confidence in the *percentage*. It is deliberately separate
 * from `quota_resets_estimated`, which is confidence in the *reset time* — a
 * Codex reading can have an exact reset for a two-day-old percentage, and a
 * Claude reading a reconstructed reset for a percentage written a minute ago.
 */

import i18n from "../i18n";

/** Wire values produced by `crate::freshness::Freshness`. */
export type Freshness = "fresh" | "aging" | "stale" | "expired" | "unknown";

/** How a row carrying this reading should be rendered. */
export interface FreshnessPresentation {
  /** Whether the percentage may be shown as the current value. */
  showsValueAsCurrent: boolean;
  /** Whether the number must be replaced by an em dash. */
  suppressesValue: boolean;
  /** Tailwind classes for the numeric value. */
  valueClass: string;
  /** Tailwind classes for the caption line under the meter. */
  captionClass: string;
  /** Short badge text, already localised. */
  badge: string;
  /** Longer caption, already localised, or null when nothing need be said. */
  caption: string | null;
}

/**
 * Elapsed time in the widget's compact two-unit style: "2 days", "3 hrs 5 min".
 *
 * Returns null for a missing or negative age so callers drop the phrase rather
 * than print a placeholder.
 */
export function formatAge(seconds: number | null): string | null {
  if (seconds === null || !Number.isFinite(seconds) || seconds < 0) {
    return null;
  }

  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);

  const unit = (count: number, key: string) => `${count} ${i18n.t(key, { count })}`;

  if (days > 0) {
    return hours > 0
      ? `${unit(days, "time.dayShort")} ${unit(hours, "time.hourShort")}`
      : unit(days, "time.dayShort");
  }
  if (hours > 0) {
    return minutes > 0
      ? `${unit(hours, "time.hourShort")} ${unit(minutes, "time.minuteShort")}`
      : unit(hours, "time.hourShort");
  }
  // Under a minute is "just now" rather than "0 min"
  return minutes > 0 ? unit(minutes, "time.minuteShort") : i18n.t("freshness.justNow");
}

/**
 * "Updated 2 days ago", or null when the age is unknown.
 *
 * Under a minute takes its own phrasing. `formatAge` returns "just now" there,
 * which is already a complete adverbial — dropping it into "Updated {{age}}
 * ago" produced "Updated just now ago", and the Thai template read no better.
 */
export function formatUpdatedAgo(ageSecs: number | null): string | null {
  if (ageSecs === null || !Number.isFinite(ageSecs) || ageSecs < 0) {
    return null;
  }
  if (ageSecs < 60) {
    return i18n.t("freshness.updatedJustNow");
  }

  const age = formatAge(ageSecs);
  return age === null ? null : i18n.t("freshness.updatedAgo", { age });
}

/**
 * Everything the UI needs to render one reading at a given freshness.
 *
 * A stale value keeps its number — throwing away the last thing we knew helps
 * nobody — but loses the confident styling and gains an explicit label, so it
 * cannot be mistaken for a current reading at a glance. An expired one loses
 * the number outright: its window has rolled over, so the figure is known to
 * be wrong rather than merely old.
 */
export function presentFreshness(
  freshness: Freshness,
  ageSecs: number | null,
): FreshnessPresentation {
  const updatedAgo = formatUpdatedAgo(ageSecs);

  switch (freshness) {
    case "fresh":
      return {
        showsValueAsCurrent: true,
        suppressesValue: false,
        valueClass: "",
        captionClass: "text-white/40",
        badge: i18n.t("freshness.fresh"),
        caption: null,
      };

    case "aging":
      return {
        showsValueAsCurrent: true,
        suppressesValue: false,
        valueClass: "",
        captionClass: "text-amber-300/70",
        badge: i18n.t("freshness.aging"),
        caption: updatedAgo,
      };

    case "stale":
      return {
        showsValueAsCurrent: false,
        suppressesValue: false,
        // Muted and italic: readable, but visibly not a live number
        valueClass: "opacity-50 italic",
        captionClass: "text-amber-400/80",
        badge: i18n.t("freshness.stale"),
        caption:
          updatedAgo === null
            ? i18n.t("freshness.lastKnown")
            : i18n.t("freshness.lastKnownAgo", { age: formatAge(ageSecs) }),
      };

    case "expired":
      return {
        showsValueAsCurrent: false,
        suppressesValue: true,
        valueClass: "opacity-50",
        captionClass: "text-white/50",
        badge: i18n.t("freshness.expired"),
        caption: i18n.t("freshness.waitingForReading"),
      };

    case "unknown":
    default:
      return {
        showsValueAsCurrent: false,
        suppressesValue: false,
        valueClass: "opacity-50 italic",
        captionClass: "text-white/50",
        badge: i18n.t("freshness.unknown"),
        caption: i18n.t("freshness.ageUnknown"),
      };
  }
}

/** Badge colours, kept beside the presentation so the two cannot drift. */
export function freshnessBadgeClass(freshness: Freshness): string {
  switch (freshness) {
    case "fresh":
      return "bg-emerald-500/15 text-emerald-300";
    case "aging":
      return "bg-amber-500/15 text-amber-300";
    case "stale":
      return "bg-amber-600/20 text-amber-200";
    case "expired":
      return "bg-red-500/15 text-red-300";
    case "unknown":
    default:
      return "bg-white/10 text-white/60";
  }
}
