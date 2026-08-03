/**
 * Dashboard current-quota behaviour, and the store contracts it depends on.
 *
 * The dashboard hides the widget, so before this panel existed opening it meant
 * losing sight of remaining quota entirely. These tests pin that it is present,
 * that it is not governed by the history range picker, and that Collect Now
 * actually refreshes it.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, act, fireEvent } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import i18n from "../i18n";
import { useAppStore } from "../store";
import type { ProviderStatus, UsageRecord, UsageSummary } from "../lib/ipc";
import { toLocalInputValue, bangkokInputToIso } from "../components/Dashboard";

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

const DAY = 86400;

const codex: ProviderStatus = {
  provider_id: "codex",
  display_name: "Codex Desktop",
  is_available: true,
  last_activity: new Date().toISOString(),
  errors: [],
  quota_fast_pct: 20,
  quota_standard_pct: 100,
  quota_excess_pct: null,
  tokens_today: null,
  quota_fast_resets_at: new Date(Date.now() + 3 * 3600_000).toISOString(),
  quota_weekly_resets_at: new Date(Date.now() + 4 * DAY * 1000).toISOString(),
  quota_resets_estimated: false,
  quota_fast_observed_at: new Date(Date.now() - 60_000).toISOString(),
  quota_weekly_observed_at: new Date(Date.now() - 2 * DAY * 1000).toISOString(),
  quota_fast_freshness: "fresh",
  quota_weekly_freshness: "stale",
  quota_fast_age_secs: 60,
  quota_weekly_age_secs: 2 * DAY,
};

const claude: ProviderStatus = {
  ...codex,
  provider_id: "claude",
  display_name: "Claude Desktop",
  quota_fast_pct: 38,
  quota_standard_pct: 12,
  quota_resets_estimated: true,
  quota_weekly_freshness: "fresh",
  quota_weekly_age_secs: 120,
  quota_weekly_observed_at: new Date(Date.now() - 120_000).toISOString(),
};

const emptyUsage: UsageSummary = {
  providers: [],
  total_tokens_today: 0,
  total_tokens_this_week: 0,
  last_updated: new Date().toISOString(),
};

const history: UsageRecord[] = [
  {
    timestamp: new Date(Date.now() - 3600_000).toISOString(),
    provider_id: "codex",
    model: "o3-mini",
    input_tokens: 100,
    output_tokens: 50,
    total_tokens: 900,
    quota_fast_pct: null,
    quota_standard_pct: null,
  },
];

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
    view: "dashboard",
    locale: "en",
  });
}

function mockDefaults(providers: ProviderStatus[] = [codex, claude]) {
  mockInvoke.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "get_current_usage":
        return Promise.resolve(emptyUsage);
      case "get_provider_status":
        return Promise.resolve(providers);
      case "get_usage_history":
        return Promise.resolve(history);
      case "trigger_collection":
        return Promise.resolve({ events_collected: 0, providers_collected: 2, errors: [] });
      default:
        return Promise.resolve(null);
    }
  });
}

async function renderDashboard() {
  const { Dashboard } = await import("../components/Dashboard");
  await act(async () => {
    render(
      <I18nextProvider i18n={i18n}>
        <Dashboard />
      </I18nextProvider>,
    );
  });
  // The name appears in the quota card and again in the history breakdown
  await waitFor(() => expect(screen.getAllByText("Codex Desktop").length).toBeGreaterThan(0));
}

describe("Dashboard current quota", () => {
  beforeEach(async () => {
    resetStore();
    mockDefaults();
    await i18n.changeLanguage("en");
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("loads provider status, which it previously never did", async () => {
    await renderDashboard();
    expect(mockInvoke.mock.calls.some((c) => c[0] === "get_provider_status")).toBe(true);
  });

  it("shows a card per provider with both windows", async () => {
    await renderDashboard();

    expect(screen.getAllByText("Codex Desktop").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Claude Desktop").length).toBeGreaterThan(0);

    // Codex: 20% used on the five-hour window leaves 80%
    expect(screen.getAllByText("80%").length).toBeGreaterThanOrEqual(1);
  });

  it("shows remaining, not consumed", async () => {
    await renderDashboard();
    // Claude weekly is 12% used → 88% left. 12% must not appear as the value.
    expect(screen.getAllByText("88%").length).toBeGreaterThanOrEqual(1);
  });

  it("badges a stale window and explains its age", async () => {
    await renderDashboard();

    expect(screen.getAllByText("Old data").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("Updated 2 days ago").length).toBeGreaterThanOrEqual(1);
  });

  it("keeps the quota panel unchanged when the history range changes", async () => {
    await renderDashboard();

    const before = mockInvoke.mock.calls.filter((c) => c[0] === "get_provider_status").length;

    // Switch the historical range to a day
    const dayButton = screen.getByText("Day");
    await act(async () => {
      dayButton.click();
    });

    const after = mockInvoke.mock.calls.filter((c) => c[0] === "get_provider_status").length;
    expect(after).toBe(before);
    // The card still shows the same current numbers
    expect(screen.getAllByText("80%").length).toBeGreaterThanOrEqual(1);
  });

  it("exposes a Collect Now action that refreshes provider status", async () => {
    await renderDashboard();

    const beforeStatus = mockInvoke.mock.calls.filter(
      (c) => c[0] === "get_provider_status",
    ).length;

    const button = screen.getByText("Collect Now");
    await act(async () => {
      button.click();
    });

    await waitFor(() => {
      expect(mockInvoke.mock.calls.some((c) => c[0] === "trigger_collection")).toBe(true);
    });
    await waitFor(() => {
      const afterStatus = mockInvoke.mock.calls.filter(
        (c) => c[0] === "get_provider_status",
      ).length;
      expect(afterStatus).toBeGreaterThan(beforeStatus);
    });
  });

  it("never fabricates reasoning or cached token totals", async () => {
    await renderDashboard();

    // `total - input - output` is not a definition of reasoning tokens, and
    // cached is not zero just because it is unavailable.
    expect(screen.getAllByText("N/A").length).toBeGreaterThanOrEqual(2);
    expect(screen.queryByText("750")).toBeNull();
  });

  it("renders Thai freshness strings without leaking keys", async () => {
    await i18n.changeLanguage("th");
    await renderDashboard();

    expect(screen.getAllByText("ข้อมูลเก่า").length).toBeGreaterThanOrEqual(1);
    expect(screen.queryByText(/freshness\./)).toBeNull();

    await i18n.changeLanguage("en");
  });
});

describe("Dashboard custom range", () => {
  beforeEach(async () => {
    resetStore();
    mockDefaults();
    await i18n.changeLanguage("en");
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  async function selectCustom() {
    await renderDashboard();
    await act(async () => {
      screen.getByText("Custom").click();
    });
  }

  it("offers real date inputs rather than a button meaning 'last 7 days'", async () => {
    await selectCustom();

    const start = document.getElementById("range-start") as HTMLInputElement;
    const end = document.getElementById("range-end") as HTMLInputElement;

    expect(start?.type).toBe("datetime-local");
    expect(end?.type).toBe("datetime-local");

    // `datetime-local` blanks its value for anything outside this exact shape,
    // so asserting only "not empty" hid *why* it could be empty. This failed on
    // CI while passing locally: the seed was built from locale formatting,
    // which is free to vary by ICU build and by the hour of day.
    expect(start.value).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/);
    expect(end.value).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/);
    expect(start.value < end.value).toBe(true);
  });

  it("round-trips a value through the Bangkok conversion unchanged", () => {
    // The two helpers are inverses; if either drifts, ranges silently shift.
    for (const hour of [0, 1, 7, 12, 17, 23]) {
      const instant = new Date(Date.UTC(2026, 6, 25, hour, 30, 0));
      const local = toLocalInputValue(instant);
      const iso = bangkokInputToIso(local);

      expect(iso, `hour ${hour}`).not.toBeNull();
      expect(new Date(iso!).getTime()).toBe(instant.getTime());
    }
  });

  it("seeds a usable value at every hour of the day", () => {
    // The failure only appeared around midnight Bangkok time, so every hour is
    // exercised rather than whichever one the suite happens to run in.
    for (let hour = 0; hour < 24; hour++) {
      const instant = new Date(Date.UTC(2026, 6, 25, hour, 30, 0));
      const el = document.createElement("input");
      el.type = "datetime-local";
      el.value = toLocalInputValue(instant);

      expect(el.value, `UTC hour ${hour} produced "${toLocalInputValue(instant)}"`).not.toBe("");
    }
  });

  it("rejects a range whose start is not before its end", async () => {
    await selectCustom();

    const before = mockInvoke.mock.calls.filter((c) => c[0] === "get_usage_history").length;

    await act(async () => {
      fireEvent.change(document.getElementById("range-start")!, {
        target: { value: "2026-08-10T10:00" },
      });
      fireEvent.change(document.getElementById("range-end")!, {
        target: { value: "2026-08-01T10:00" },
      });
    });

    // Visible complaint, and no silent fall back to a seven-day window
    expect(screen.getByRole("alert").textContent).toBe(
      "The start must be before the end",
    );
    const after = mockInvoke.mock.calls.filter((c) => c[0] === "get_usage_history").length;
    expect(after).toBe(before);
  });

  it("queries the range the user actually typed", async () => {
    await selectCustom();

    await act(async () => {
      fireEvent.change(document.getElementById("range-start")!, {
        target: { value: "2026-07-20T00:00" },
      });
      fireEvent.change(document.getElementById("range-end")!, {
        target: { value: "2026-07-25T00:00" },
      });
    });

    await waitFor(() => {
      const call = mockInvoke.mock.calls.filter((c) => c[0] === "get_usage_history").pop();
      expect(call).toBeTruthy();
      const args = call![1] as { start: string; end: string };
      // Bangkok is UTC+7, so 00:00 local is 17:00 the previous day in UTC
      expect(args.start).toBe("2026-07-19T17:00:00.000Z");
      expect(args.end).toBe("2026-07-24T17:00:00.000Z");
    });

    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("store request-specific loading", () => {
  beforeEach(() => {
    resetStore();
    mockDefaults();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("does not let one request clear another's loading flag", async () => {
    // The widget fires both every 10s. With one shared boolean, whichever
    // resolved first cleared the flag for both.
    let releaseUsage: (v: UsageSummary) => void = () => {};
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "get_current_usage") {
        return new Promise<UsageSummary>((resolve) => {
          releaseUsage = resolve;
        });
      }
      if (cmd === "get_provider_status") return Promise.resolve([codex]);
      return Promise.resolve(null);
    });

    const store = useAppStore.getState();
    const usagePromise = store.fetchUsage();
    await act(async () => {
      await store.fetchProviderStatus();
    });

    // Provider status finished; usage is still in flight
    expect(useAppStore.getState().providerStatusLoading).toBe(false);
    expect(useAppStore.getState().usageLoading).toBe(true);

    await act(async () => {
      releaseUsage(emptyUsage);
      await usagePromise;
    });
    expect(useAppStore.getState().usageLoading).toBe(false);
  });

  it("reports both failures when a collection refresh half-fails", async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "trigger_collection") {
        return Promise.resolve({ events_collected: 0, providers_collected: 0, errors: [] });
      }
      if (cmd === "get_current_usage") return Promise.reject(new Error("usage boom"));
      if (cmd === "get_provider_status") return Promise.resolve([codex]);
      return Promise.resolve(null);
    });

    await act(async () => {
      await useAppStore.getState().triggerCollection();
    });

    const state = useAppStore.getState();
    // The half that worked still landed
    expect(state.providers).toHaveLength(1);
    expect(state.error).toContain("usage boom");
    expect(state.collectionLoading).toBe(false);
  });
});
