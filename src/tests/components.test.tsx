/**
 * Frontend component tests for AI Usage Widget.
 *
 * Tests CompactWidget, Dashboard, locale switching, and "Not available" display.
 * Mocks @tauri-apps/api/core to avoid needing a real Tauri runtime.
 *
 * Validates: Requirements 6.1, 6.2, 6.3, 7.1, 9.1, 9.2
 */

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import i18n from "../i18n";
import { useAppStore } from "../store";
import type { UsageSummary, ProviderStatus, UsageRecord } from "../lib/ipc";

// ─── Mock @tauri-apps/api/core ──────────────────────────────────────────────────

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// ─── Mock recharts to avoid SVG rendering issues in jsdom ───────────────────────

vi.mock("recharts", () => {
  const MockResponsiveContainer = ({ children }: { children: React.ReactNode }) => (
    <div data-testid="responsive-container">{children}</div>
  );
  const MockLineChart = ({ children }: { children: React.ReactNode }) => (
    <div data-testid="line-chart">{children}</div>
  );
  const MockLine = () => <div data-testid="chart-line" />;
  const MockXAxis = () => <div />;
  const MockYAxis = () => <div />;
  const MockCartesianGrid = () => <div />;
  const MockTooltip = () => <div />;
  const MockLegend = () => <div />;

  return {
    ResponsiveContainer: MockResponsiveContainer,
    LineChart: MockLineChart,
    Line: MockLine,
    XAxis: MockXAxis,
    YAxis: MockYAxis,
    CartesianGrid: MockCartesianGrid,
    Tooltip: MockTooltip,
    Legend: MockLegend,
  };
});

// ─── Fixture Data ───────────────────────────────────────────────────────────────

const fixtureUsageSummary: UsageSummary = {
  providers: [
    {
      provider_id: "codex",
      input_tokens_today: 15000,
      output_tokens_today: 8000,
      total_tokens_today: 23000,
      input_tokens_this_week: 75000,
      output_tokens_this_week: 40000,
      total_tokens_this_week: 115000,
      last_activity: new Date(Date.now() - 120000).toISOString(),
    },
    {
      provider_id: "claude",
      input_tokens_today: 5000,
      output_tokens_today: 3000,
      total_tokens_today: 8000,
      input_tokens_this_week: 25000,
      output_tokens_this_week: 15000,
      total_tokens_this_week: 40000,
      last_activity: new Date(Date.now() - 300000).toISOString(),
    },
  ],
  total_tokens_today: 31000,
  total_tokens_this_week: 155000,
  last_updated: new Date().toISOString(),
};

const fixtureProviders: ProviderStatus[] = [
  {
    provider_id: "codex",
    display_name: "Codex Desktop",
    is_available: true,
    last_collection: new Date().toISOString(),
    events_collected: 42,
    errors: [],
    quota_fast_pct: null,
    quota_standard_pct: null,
    quota_excess_pct: null,
    tokens_today: null,
    quota_resets_at: null,
  },
  {
    provider_id: "claude",
    display_name: "Claude Desktop",
    is_available: true,
    last_collection: new Date().toISOString(),
    events_collected: 18,
    errors: [],
    quota_fast_pct: 82.5,
    quota_standard_pct: 41.0,
    quota_excess_pct: null,
    tokens_today: 29036,
    quota_resets_at: null,
  },
];

const fixtureHistory: UsageRecord[] = [
  {
    timestamp: new Date(Date.now() - 6 * 3600000).toISOString(),
    provider_id: "codex",
    model: "o3-mini",
    input_tokens: 5000,
    output_tokens: 3000,
    total_tokens: 8000,
    quota_fast_pct: null,
    quota_standard_pct: null,
  },
  {
    timestamp: new Date(Date.now() - 3 * 3600000).toISOString(),
    provider_id: "codex",
    model: "o3-mini",
    input_tokens: 10000,
    output_tokens: 5000,
    total_tokens: 15000,
    quota_fast_pct: null,
    quota_standard_pct: null,
  },
  {
    timestamp: new Date(Date.now() - 1 * 3600000).toISOString(),
    provider_id: "claude",
    model: "claude-sonnet-4",
    input_tokens: 5000,
    output_tokens: 3000,
    total_tokens: 8000,
    quota_fast_pct: 45.5,
    quota_standard_pct: 20.0,
  },
];

// ─── Helper ─────────────────────────────────────────────────────────────────────

function setupMocks() {
  mockInvoke.mockImplementation((cmd: string) => {
    switch (cmd) {
      case "get_current_usage":
        return Promise.resolve(fixtureUsageSummary);
      case "get_provider_status":
        return Promise.resolve(fixtureProviders);
      case "get_usage_history":
        return Promise.resolve(fixtureHistory);
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
    isLoading: false,
    error: null,
    view: "compact",
    locale: "th",
  });
}

// ─── Tests ──────────────────────────────────────────────────────────────────────

describe("CompactWidget", () => {
  beforeEach(() => {
    setupMocks();
    vi.useFakeTimers({ shouldAdvanceTime: true });
    resetStore();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it("renders provider data correctly", async () => {
    const { CompactWidget } = await import("../components/CompactWidget");

    await act(async () => {
      render(
        <I18nextProvider i18n={i18n}>
          <CompactWidget />
        </I18nextProvider>,
      );
    });

    // Wait for data to load and display
    await waitFor(() => {
      expect(screen.getByText("Codex Desktop")).toBeTruthy();
    });

    // Verify provider names rendered
    expect(screen.getByText("Codex Desktop")).toBeTruthy();
    expect(screen.getByText("Claude Desktop")).toBeTruthy();

    // Token counts render compact (23,000 -> "23.0K") so they fit 340px
    const codexTokens = screen.getAllByText("23.0K");
    expect(codexTokens.length).toBeGreaterThanOrEqual(1);
    const claudeTokens = screen.getAllByText("8.0K");
    expect(claudeTokens.length).toBeGreaterThanOrEqual(1);
  });
});

describe("Dashboard", () => {
  beforeEach(() => {
    setupMocks();
    vi.useFakeTimers({ shouldAdvanceTime: true });
    resetStore();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it("displays charts with correct data", async () => {
    const { Dashboard } = await import("../components/Dashboard");

    await act(async () => {
      render(
        <I18nextProvider i18n={i18n}>
          <Dashboard />
        </I18nextProvider>,
      );
    });

    // Wait for the usage history title to render (appears as h1 and h2)
    await waitFor(() => {
      const headings = screen.getAllByText("ประวัติการใช้งาน");
      expect(headings.length).toBeGreaterThanOrEqual(1);
    });

    // The chart container should render (mocked ResponsiveContainer)
    await waitFor(() => {
      expect(screen.getByTestId("responsive-container")).toBeTruthy();
    });

    // Verify the line chart rendered inside
    expect(screen.getByTestId("line-chart")).toBeTruthy();
  });
});

describe("Locale switching", () => {
  beforeEach(() => {
    setupMocks();
    vi.useFakeTimers({ shouldAdvanceTime: true });
    resetStore();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
    // Reset to Thai default
    act(() => {
      i18n.changeLanguage("th");
    });
  });

  it("updates all text when locale changes", async () => {
    const { CompactWidget } = await import("../components/CompactWidget");

    // Start with Thai
    await act(async () => {
      await i18n.changeLanguage("th");
    });

    const { rerender } = await act(async () => {
      return render(
        <I18nextProvider i18n={i18n}>
          <CompactWidget />
        </I18nextProvider>,
      );
    });

    // Wait for data load
    await waitFor(() => {
      expect(screen.getByText("Codex Desktop")).toBeTruthy();
    });

    // Thai footer label ("อัปเดตล่าสุด") should appear
    const thaiTexts = screen.getAllByText(/อัปเดตล่าสุด/);
    expect(thaiTexts.length).toBeGreaterThan(0);

    // Switch to English
    await act(async () => {
      await i18n.changeLanguage("en");
    });

    // Re-render to pick up language change
    await act(async () => {
      rerender(
        <I18nextProvider i18n={i18n}>
          <CompactWidget />
        </I18nextProvider>,
      );
    });

    // English footer label should now appear instead
    const enTexts = screen.getAllByText(/Last updated/);
    expect(enTexts.length).toBeGreaterThan(0);
  });
});

describe("Not available display", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });

    // Mock with one provider unavailable
    mockInvoke.mockImplementation((cmd: string) => {
      switch (cmd) {
        case "get_current_usage":
          return Promise.resolve({
            ...fixtureUsageSummary,
            providers: [fixtureUsageSummary.providers[0]],
          });
        case "get_provider_status":
          return Promise.resolve([
            fixtureProviders[0],
            {
              ...fixtureProviders[1],
              is_available: false,
            },
          ]);
        case "get_usage_history":
          return Promise.resolve(fixtureHistory);
        default:
          return Promise.resolve(null);
      }
    });

    resetStore();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
    act(() => {
      i18n.changeLanguage("th");
    });
  });

  it("shows 'ไม่มีข้อมูล' for unavailable providers in Thai locale", async () => {
    const { CompactWidget } = await import("../components/CompactWidget");

    await act(async () => {
      await i18n.changeLanguage("th");
    });

    await act(async () => {
      render(
        <I18nextProvider i18n={i18n}>
          <CompactWidget />
        </I18nextProvider>,
      );
    });

    // Wait for data
    await waitFor(() => {
      expect(screen.getByText("Codex Desktop")).toBeTruthy();
    });

    // Claude should show "ไม่มีข้อมูล" since it's unavailable
    expect(screen.getByText("ไม่มีข้อมูล")).toBeTruthy();
  });

  it("shows 'Not available' for unavailable providers in English locale", async () => {
    const { CompactWidget } = await import("../components/CompactWidget");

    await act(async () => {
      await i18n.changeLanguage("en");
    });

    await act(async () => {
      render(
        <I18nextProvider i18n={i18n}>
          <CompactWidget />
        </I18nextProvider>,
      );
    });

    // Wait for data
    await waitFor(() => {
      expect(screen.getByText("Codex Desktop")).toBeTruthy();
    });

    // Claude should show "Not available" since it's unavailable
    expect(screen.getByText("Not available")).toBeTruthy();
  });
});
