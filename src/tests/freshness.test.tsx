/**
 * Quota freshness in the UI.
 *
 * The failure these guard against is silent: a well-formed percentage from two
 * days ago rendering exactly like one from a minute ago. Every assertion here
 * is about a stale reading being *visibly* different from a live one.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import i18n from "../i18n";
import { useAppStore } from "../store";
import type { ProviderStatus, UsageSummary } from "../lib/ipc";
import type { Freshness } from "../lib/freshness";
import { presentFreshness, formatAge, formatUpdatedAgo } from "../lib/freshness";
import { bindingWindow, quotaWindows } from "../lib/quota";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("recharts", () => {
  const Passthrough = ({ children }: { children?: React.ReactNode }) => <div>{children}</div>;
  return {
    ResponsiveContainer: Passthrough,
    LineChart: Passthrough,
    Line: () => <div />,
    XAxis: () => <div />,
    YAxis: () => <div />,
    CartesianGrid: () => <div />,
    Tooltip: () => <div />,
    Legend: () => <div />,
  };
});

const HOUR = 3600;
const DAY = 86400;

/** A provider whose weekly window carries the given freshness. */
function providerWith(
  freshness: Freshness,
  ageSecs: number | null,
  overrides: Partial<ProviderStatus> = {},
): ProviderStatus {
  return {
    provider_id: "codex",
    display_name: "Codex Desktop",
    is_available: true,
    last_activity: new Date().toISOString(),
    errors: [],
    quota_fast_pct: null,
    quota_standard_pct: 40,
    quota_excess_pct: null,
    tokens_today: null,
    quota_fast_resets_at: null,
    quota_weekly_resets_at: new Date(Date.now() + 4 * DAY * 1000).toISOString(),
    quota_resets_estimated: false,
    quota_fast_observed_at: null,
    quota_weekly_observed_at:
      ageSecs === null ? null : new Date(Date.now() - ageSecs * 1000).toISOString(),
    quota_fast_freshness: "unknown",
    quota_weekly_freshness: freshness,
    quota_fast_age_secs: null,
    quota_weekly_age_secs: ageSecs,
    ...overrides,
  };
}

const emptyUsage: UsageSummary = {
  providers: [],
  total_tokens_today: 0,
  total_tokens_this_week: 0,
  last_updated: new Date().toISOString(),
};

function mockWith(providers: ProviderStatus[]) {
  mockInvoke.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "get_current_usage":
        return Promise.resolve(emptyUsage);
      case "get_provider_status":
        return Promise.resolve(providers);
      case "get_usage_history":
        return Promise.resolve([]);
      case "get_widget_collapsed":
        return Promise.resolve(false);
      default:
        return Promise.resolve(null);
    }
  });
}

function resetStore() {
  useAppStore.setState({
    usage: null,
    providers: [],
    history: [],
    usageLoading: false,
    providerStatusLoading: false,
    historyLoading: false,
    collectionLoading: false,
    settingsLoading: false,
    error: null,
    view: "compact",
    locale: "th",
  });
}

async function renderWidget() {
  const { CompactWidget } = await import("../components/CompactWidget");
  await act(async () => {
    render(
      <I18nextProvider i18n={i18n}>
        <CompactWidget />
      </I18nextProvider>,
    );
  });
  await waitFor(() => expect(screen.getByText("Codex Desktop")).toBeTruthy());
}

// ─── The pure presentation layer ────────────────────────────────────────────

describe("presentFreshness", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
  });

  it("lets only fresh and aging readings stand as current", () => {
    expect(presentFreshness("fresh", 60).showsValueAsCurrent).toBe(true);
    expect(presentFreshness("aging", HOUR).showsValueAsCurrent).toBe(true);
    expect(presentFreshness("stale", 2 * DAY).showsValueAsCurrent).toBe(false);
    expect(presentFreshness("expired", 60).showsValueAsCurrent).toBe(false);
    expect(presentFreshness("unknown", null).showsValueAsCurrent).toBe(false);
  });

  it("suppresses the number only when the window has ended", () => {
    // Stale keeps its value — the last thing we knew is still worth showing.
    // Expired does not: that figure describes a window that no longer exists.
    expect(presentFreshness("stale", 2 * DAY).suppressesValue).toBe(false);
    expect(presentFreshness("expired", 60).suppressesValue).toBe(true);
  });

  it("labels a stale reading as last known, with its age", () => {
    const look = presentFreshness("stale", 2 * DAY);
    expect(look.caption).toBe("Last known, updated 2 days ago");
    expect(look.valueClass).toContain("opacity-50");
  });

  it("tells the reader an expired window is waiting for a new reading", () => {
    expect(presentFreshness("expired", 60).caption).toBe("Waiting for a new quota reading");
  });

  it("says the age is unknown rather than implying it is new", () => {
    const look = presentFreshness("unknown", null);
    expect(look.caption).toBe("Reading age unknown");
    expect(look.showsValueAsCurrent).toBe(false);
  });

  it("gives an aging reading its age but no alarm", () => {
    expect(presentFreshness("aging", 2 * HOUR).caption).toBe("Updated 2 hrs ago");
  });

  it("says nothing extra for a fresh reading", () => {
    expect(presentFreshness("fresh", 30).caption).toBeNull();
  });
});

describe("formatUpdatedAgo", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
  });

  it("does not read 'Updated just now ago' under a minute", () => {
    // "just now" is already a complete adverbial, so interpolating it into
    // "Updated {{age}} ago" produced a sentence nobody would write.
    expect(formatUpdatedAgo(30)).toBe("Updated just now");
    expect(formatUpdatedAgo(59)).toBe("Updated just now");
    expect(formatUpdatedAgo(0)).toBe("Updated just now");
  });

  it("switches to the interpolated form from a minute upward", () => {
    expect(formatUpdatedAgo(60)).toBe("Updated 1 min ago");
    expect(formatUpdatedAgo(2 * HOUR)).toBe("Updated 2 hrs ago");
    expect(formatUpdatedAgo(2 * DAY)).toBe("Updated 2 days ago");
  });

  it("refuses to invent an age", () => {
    expect(formatUpdatedAgo(null)).toBeNull();
    expect(formatUpdatedAgo(-5)).toBeNull();
    expect(formatUpdatedAgo(Number.NaN)).toBeNull();
  });

  it("never emits the doubled phrase at any age, in either language", async () => {
    for (const lang of ["en", "th"]) {
      await i18n.changeLanguage(lang);

      // The exact string the old code produced: the "just now" adverbial fed
      // through the "{{age}} ago" template. Built from the templates rather
      // than hardcoded, so it stays correct if either translation changes.
      const broken = i18n.t("freshness.updatedAgo", {
        age: i18n.t("freshness.justNow"),
      });

      for (const secs of [0, 1, 30, 59, 60, 61, 3599, 3600, DAY, 3 * DAY]) {
        expect(formatUpdatedAgo(secs)).not.toBe(broken);
      }
    }
    await i18n.changeLanguage("en");
  });

  it("reads naturally in Thai too", async () => {
    await i18n.changeLanguage("th");
    expect(formatUpdatedAgo(30)).toBe("อัปเดตเมื่อครู่");
    expect(formatUpdatedAgo(2 * DAY)).toBe("อัปเดตเมื่อ 2 วัน ที่แล้ว");
    await i18n.changeLanguage("en");
  });
});

describe("formatAge", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
  });

  it("renders at most two units", () => {
    expect(formatAge(2 * DAY)).toBe("2 days");
    expect(formatAge(DAY + 3 * HOUR)).toBe("1 day 3 hrs");
    expect(formatAge(2 * HOUR + 5 * 60)).toBe("2 hrs 5 min");
    expect(formatAge(45)).toBe("just now");
  });

  it("refuses to invent an age", () => {
    expect(formatAge(null)).toBeNull();
    expect(formatAge(-5)).toBeNull();
  });

  it("translates to Thai without leaking keys", async () => {
    await i18n.changeLanguage("th");
    expect(formatAge(2 * DAY)).toBe("2 วัน");
    expect(presentFreshness("stale", 2 * DAY).caption).toBe("ข้อมูลเก่า 2 วัน ที่แล้ว");
    expect(presentFreshness("expired", 60).caption).toBe("รอค่าโควตาใหม่");
    await i18n.changeLanguage("en");
  });
});

// ─── Window shaping ─────────────────────────────────────────────────────────

describe("bindingWindow", () => {
  it("names the window that is actually constraining the user", () => {
    const provider = providerWith("fresh", 60, {
      quota_fast_pct: 20,
      quota_standard_pct: 95,
      quota_fast_freshness: "fresh",
      quota_fast_age_secs: 60,
    });

    const binding = bindingWindow(provider);
    expect(binding?.labelKey).toBe("quota.standard");
    expect(binding?.usedPct).toBe(95);
  });

  it("skips an expired window rather than pinning the summary to it", () => {
    // 100% on a window that has already rolled over is known to be wrong;
    // reporting it as the binding limit would be the worst possible summary.
    const provider = providerWith("expired", 60, {
      quota_fast_pct: 30,
      quota_standard_pct: 100,
      quota_fast_freshness: "fresh",
      quota_fast_age_secs: 60,
    });

    expect(bindingWindow(provider)?.labelKey).toBe("quota.fastHours");
  });

  it("returns null when nothing is reported", () => {
    const provider = providerWith("unknown", null, {
      quota_fast_pct: null,
      quota_standard_pct: null,
    });
    expect(bindingWindow(provider)).toBeNull();
  });

  it("carries per-window freshness through to the rows", () => {
    const provider = providerWith("stale", 2 * DAY);
    const windows = quotaWindows(provider);

    expect(windows[0].freshness).toBe("unknown");
    expect(windows[1].freshness).toBe("stale");
    expect(windows[1].ageSecs).toBe(2 * DAY);
  });
});

// ─── Widget rendering ───────────────────────────────────────────────────────

describe("CompactWidget freshness rendering", () => {
  beforeEach(async () => {
    resetStore();
    await i18n.changeLanguage("en");
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("marks a two-day-old reading as last known rather than current", async () => {
    mockWith([providerWith("stale", 2 * DAY)]);
    await renderWidget();

    expect(screen.getByText("Last known, updated 2 days ago")).toBeTruthy();
  });

  it("replaces an expired percentage with a dash and an explanation", async () => {
    mockWith([providerWith("expired", 120)]);
    await renderWidget();

    expect(screen.getByText("Waiting for a new quota reading")).toBeTruthy();
    // 40% used would otherwise render as "60%"
    expect(screen.queryByText("60%")).toBeNull();
  });

  it("shows an aging reading's age beside its value", async () => {
    mockWith([providerWith("aging", 2 * HOUR)]);
    await renderWidget();

    expect(screen.getByText("Updated 2 hrs ago")).toBeTruthy();
    expect(screen.getByText("60%")).toBeTruthy();
  });

  it("adds no age caption to a fresh reading", async () => {
    mockWith([providerWith("fresh", 30)]);
    await renderWidget();

    expect(screen.getByText("60%")).toBeTruthy();
    expect(screen.queryByText(/Last known/)).toBeNull();
    expect(screen.queryByText(/Updated/)).toBeNull();
  });

  it("labels an unknown age instead of implying the value is new", async () => {
    mockWith([providerWith("unknown", null)]);
    await renderWidget();

    // Both windows are unknown in this fixture — the label belongs on each,
    // because each is a separate reading with a separate age.
    expect(screen.getAllByText("Reading age unknown")).toHaveLength(2);
  });

  it("offers the exact source time on hover", async () => {
    const observed = new Date(Date.now() - 2 * DAY * 1000).toISOString();
    mockWith([providerWith("stale", 2 * DAY, { quota_weekly_observed_at: observed })]);
    await renderWidget();

    const caption = screen.getByText("Last known, updated 2 days ago");
    expect(caption.getAttribute("title")).toContain("Reading taken");
  });
});

// ─── Collapsed summary ──────────────────────────────────────────────────────

describe("collapsed binding-window label", () => {
  beforeEach(async () => {
    resetStore();
    await i18n.changeLanguage("en");
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  async function renderCollapsed(provider: ProviderStatus) {
    mockInvoke.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "get_current_usage":
          return Promise.resolve(emptyUsage);
        case "get_provider_status":
          return Promise.resolve([provider]);
        case "get_widget_collapsed":
          return Promise.resolve(true);
        default:
          return Promise.resolve(null);
      }
    });

    const { CompactWidget } = await import("../components/CompactWidget");
    await act(async () => {
      render(
        <I18nextProvider i18n={i18n}>
          <CompactWidget />
        </I18nextProvider>,
      );
    });
    await waitFor(() => {
      const toggle = document.querySelector("button[aria-expanded]");
      expect(toggle?.getAttribute("aria-expanded")).toBe("false");
    });
  }

  it("names the binding window, not just a bare percentage", async () => {
    // "CX 0%" cannot say whether the wait is hours or days
    await renderCollapsed(
      providerWith("fresh", 60, {
        quota_fast_pct: 76,
        quota_standard_pct: 100,
        quota_fast_freshness: "fresh",
        quota_fast_age_secs: 60,
      }),
    );

    expect(screen.getByText(/CX W 0%/)).toBeTruthy();
  });

  it("uses the five-hour code when that window is the binding one", async () => {
    await renderCollapsed(
      providerWith("fresh", 60, {
        quota_fast_pct: 76,
        quota_standard_pct: 10,
        quota_fast_freshness: "fresh",
        quota_fast_age_secs: 60,
      }),
    );

    expect(screen.getByText(/CX 5h 24%/)).toBeTruthy();
  });

  it("does not put 'just now ago' in the collapsed tooltip", async () => {
    // The tooltip used to build the phrase itself instead of calling the
    // shared helper, so it reproduced the bug independently.
    await renderCollapsed(providerWith("fresh", 20));

    const chip = screen.getByText(/CX W/);
    expect(chip.getAttribute("title")).toContain("Updated just now");
    expect(chip.getAttribute("title")).not.toContain("just now ago");
  });

  it("flags a stale binding reading and explains it on hover", async () => {
    await renderCollapsed(providerWith("stale", 2 * DAY));

    const chip = screen.getByText(/CX W/);
    expect(chip.textContent).toContain("•");
    expect(chip.getAttribute("title")).toContain("Last known");
  });
});
