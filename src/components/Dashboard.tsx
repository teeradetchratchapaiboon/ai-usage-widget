/**
 * Dashboard - Expanded dashboard view (900×600px window).
 *
 * Displays usage history charts with configurable time range and granularity,
 * per-provider breakdowns with model-level detail, and token type breakdown.
 * Uses Recharts for data visualization.
 */

import { useEffect, useState, useMemo, useCallback } from "react";
import { useTranslation } from "react-i18next";
import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
} from "recharts";
import { useAppStore } from "../store";
import { QuotaOverview } from "./QuotaOverview";
import { formatTokenCount, formatBangkokTime } from "../lib/format";
import type { UsageRecord } from "../lib/ipc";

// ─── Types ──────────────────────────────────────────────────────────────────────

type TimeRange = "day" | "week" | "month" | "custom";
type Granularity = "hourly" | "daily" | "weekly" | "monthly";

interface ChartDataPoint {
  timestamp: string;
  label: string;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

interface ProviderBreakdown {
  providerId: string;
  model: string;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  records: number;
}

interface TokenTypeSummary {
  input: number;
  output: number;
  /** Null when the stored schema cannot supply it — never a guess. */
  reasoning: number | null;
  cached: number | null;
  total: number;
}

// ─── Helpers ────────────────────────────────────────────────────────────────────

/** Picker value that applies no provider filter. */
/** History is charted in Bangkok time; the custom range inputs match it. */
const DISPLAY_TIME_ZONE = "Asia/Bangkok";

const ALL_PROVIDERS = "__all__";

/** Translated provider name, falling back to the raw id. */
function providerLabel(t: (key: string) => string, providerId: string): string {
  if (providerId === "codex") return t("provider.codex");
  if (providerId === "claude") return t("provider.claude");
  return providerId;
}

/** A user-entered range, as the two `datetime-local` fields hold it. */
interface CustomRange {
  start: string;
  end: string;
}

/** Wall-clock reading of an instant in a given zone, as plain numbers. */
interface ZonedParts {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
  second: number;
}

/**
 * Read an instant as wall-clock numbers in `timeZone`.
 *
 * The calendar, numbering system and hour cycle are all pinned rather than
 * left to the locale. `datetime-local` accepts exactly "YYYY-MM-DDTHH:mm" and
 * silently blanks the field for anything else, so every way ICU is allowed to
 * vary — "24" for midnight under an h24 cycle, unpadded components, non-Latin
 * digits, a non-Gregorian calendar — is a way for the input to come up empty
 * on someone else's machine. Numbers are extracted here and the string is
 * assembled by [`toLocalInputValue`], so formatting can no longer decide
 * whether the field works.
 */
function zonedParts(date: Date, timeZone: string): ZonedParts | null {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    calendar: "gregory",
    numberingSystem: "latn",
    hourCycle: "h23",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).formatToParts(date);

  const read = (type: string): number => {
    const raw = parts.find((p) => p.type === type)?.value;
    return raw === undefined ? Number.NaN : Number(raw);
  };

  const out: ZonedParts = {
    year: read("year"),
    month: read("month"),
    day: read("day"),
    // h23 should never yield 24, but a stray one must wrap rather than
    // produce a value the input rejects outright
    hour: read("hour") % 24,
    minute: read("minute"),
    second: read("second"),
  };

  return Object.values(out).every(Number.isFinite) ? out : null;
}

const pad = (value: number, width = 2) => String(value).padStart(width, "0");

/**
 * `datetime-local` wants "YYYY-MM-DDTHH:mm" in the *viewer's* zone.
 *
 * The dashboard renders its history in Asia/Bangkok, so the inputs are seeded
 * in that zone too — a range typed in one zone and charted in another silently
 * shifts every bucket.
 *
 * Returns "" only when the instant itself is unusable, which the caller can
 * distinguish from a value the browser rejected.
 */
export function toLocalInputValue(date: Date): string {
  const p = zonedParts(date, DISPLAY_TIME_ZONE);
  if (p === null) return "";

  return `${pad(p.year, 4)}-${pad(p.month)}-${pad(p.day)}T${pad(p.hour)}:${pad(p.minute)}`;
}

/** Seeded to the last seven days so the fields are never blank. */
const defaultCustomRange: CustomRange = {
  start: toLocalInputValue(new Date(Date.now() - 7 * 24 * 60 * 60 * 1000)),
  end: toLocalInputValue(new Date()),
};

/**
 * Interpret a `datetime-local` value as an instant in Asia/Bangkok.
 *
 * `new Date("2026-08-01T10:00")` uses the machine's zone, which is only
 * correct by luck. The offset is measured against the target zone instead.
 */
export function bangkokInputToIso(value: string): string | null {
  if (!value) return null;

  const naive = new Date(`${value}:00Z`);
  if (Number.isNaN(naive.getTime())) return null;

  // What the target zone calls that same instant, read back as numbers. The
  // previous version reformatted to a locale string and unpicked it with a
  // regex, which assumed US date order and Latin digits.
  const p = zonedParts(naive, DISPLAY_TIME_ZONE);
  if (p === null) return null;

  const asZoned = Date.UTC(p.year, p.month - 1, p.day, p.hour, p.minute, p.second);
  const offsetMs = asZoned - naive.getTime();

  return new Date(naive.getTime() - offsetMs).toISOString();
}

/** Resolved bounds for a range, or null when a custom range is unusable. */
function resolveRange(
  range: TimeRange,
  custom: CustomRange,
): { start: string; end: string } | null {
  const now = new Date();
  const ago = (ms: number) => new Date(now.getTime() - ms).toISOString();

  switch (range) {
    case "day":
      return { start: ago(24 * 60 * 60 * 1000), end: now.toISOString() };
    case "week":
      return { start: ago(7 * 24 * 60 * 60 * 1000), end: now.toISOString() };
    case "month":
      return { start: ago(30 * 24 * 60 * 60 * 1000), end: now.toISOString() };
    case "custom": {
      const start = bangkokInputToIso(custom.start);
      const end = bangkokInputToIso(custom.end);
      if (start === null || end === null || start >= end) return null;
      return { start, end };
    }
  }
}

/** True when the custom fields cannot produce a usable range. */
function customRangeError(custom: CustomRange): boolean {
  return resolveRange("custom", custom) === null;
}

function formatChartLabel(isoTimestamp: string, granularity: Granularity): string {
  const date = new Date(isoTimestamp);
  if (isNaN(date.getTime())) return isoTimestamp;

  switch (granularity) {
    case "hourly":
      return date.toLocaleTimeString("en-US", {
        timeZone: "Asia/Bangkok",
        hour: "2-digit",
        minute: "2-digit",
        hour12: false,
      });
    case "daily":
      return date.toLocaleDateString("en-US", {
        timeZone: "Asia/Bangkok",
        month: "short",
        day: "numeric",
      });
    case "weekly":
      return date.toLocaleDateString("en-US", {
        timeZone: "Asia/Bangkok",
        month: "short",
        day: "numeric",
      });
    case "monthly":
      return date.toLocaleDateString("en-US", {
        timeZone: "Asia/Bangkok",
        month: "short",
        year: "2-digit",
      });
  }
}

function aggregateChartData(
  history: UsageRecord[],
  granularity: Granularity,
): ChartDataPoint[] {
  const grouped = new Map<string, ChartDataPoint>();

  for (const record of history) {
    const label = formatChartLabel(record.timestamp, granularity);
    const key = record.timestamp;

    const existing = grouped.get(key);
    if (existing) {
      existing.inputTokens += record.input_tokens ?? 0;
      existing.outputTokens += record.output_tokens ?? 0;
      existing.totalTokens += record.total_tokens ?? 0;
    } else {
      grouped.set(key, {
        timestamp: record.timestamp,
        label,
        inputTokens: record.input_tokens ?? 0,
        outputTokens: record.output_tokens ?? 0,
        totalTokens: record.total_tokens ?? 0,
      });
    }
  }

  return Array.from(grouped.values()).sort(
    (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime(),
  );
}

function computeProviderBreakdowns(history: UsageRecord[]): ProviderBreakdown[] {
  const map = new Map<string, ProviderBreakdown>();

  for (const record of history) {
    const key = `${record.provider_id}::${record.model ?? "unknown"}`;
    const existing = map.get(key);

    if (existing) {
      existing.inputTokens += record.input_tokens ?? 0;
      existing.outputTokens += record.output_tokens ?? 0;
      existing.totalTokens += record.total_tokens ?? 0;
      existing.records += 1;
    } else {
      map.set(key, {
        providerId: record.provider_id,
        model: record.model ?? "unknown",
        inputTokens: record.input_tokens ?? 0,
        outputTokens: record.output_tokens ?? 0,
        totalTokens: record.total_tokens ?? 0,
        records: 1,
      });
    }
  }

  return Array.from(map.values()).sort((a, b) => b.totalTokens - a.totalTokens);
}

function computeTokenTypeSummary(history: UsageRecord[]): TokenTypeSummary {
  let input = 0;
  let output = 0;
  let total = 0;

  for (const record of history) {
    input += record.input_tokens ?? 0;
    output += record.output_tokens ?? 0;
    total += record.total_tokens ?? 0;
  }

  // Reasoning and cached are not carried by UsageRecord over IPC.
  //
  // `total - input - output` is not a definition of reasoning tokens — the
  // aggregate rows do not promise that identity, and cached input is counted
  // inside `input` for Codex, so the remainder is whatever the arithmetic
  // happens to leave. Reporting it as "reasoning", or cached as a confident
  // zero, invents figures the user could act on. They are reported as
  // unavailable until the query layer aggregates the real columns.
  return { input, output, reasoning: null, cached: null, total };
}

// ─── Component ──────────────────────────────────────────────────────────────────

export function Dashboard() {
  const { t } = useTranslation();
  const history = useAppStore((s) => s.history);
  const historyLoading = useAppStore((s) => s.historyLoading);
  const error = useAppStore((s) => s.error);
  const fetchHistory = useAppStore((s) => s.fetchHistory);
  const fetchProviderStatus = useAppStore((s) => s.fetchProviderStatus);

  const [timeRange, setTimeRange] = useState<TimeRange>("week");
  const [granularity, setGranularity] = useState<Granularity>("daily");
  const [provider, setProvider] = useState<string>(ALL_PROVIDERS);
  const [customRange, setCustomRange] = useState<CustomRange>(defaultCustomRange);

  // Fetch history when time range or granularity changes
  const loadHistory = useCallback(() => {
    const bounds = resolveRange(timeRange, customRange);
    if (bounds === null) return; // invalid custom range: keep the last good chart
    fetchHistory(bounds.start, bounds.end, granularity);
  }, [timeRange, customRange, granularity, fetchHistory]);

  useEffect(() => {
    loadHistory();
  }, [loadHistory]);

  // Current quota is loaded on its own schedule. It is deliberately not tied
  // to the range picker below: that picker chooses a period to look *back*
  // over, and a current reading has no period to be filtered by.
  useEffect(() => {
    fetchProviderStatus();
    const interval = setInterval(() => fetchProviderStatus(), 30_000);
    return () => clearInterval(interval);
  }, [fetchProviderStatus]);

  // Which providers the current range has data for, so the picker only ever
  // offers something that can actually be charted
  const availableProviders = useMemo(
    () => Array.from(new Set(history.map((r) => r.provider_id))).sort(),
    [history],
  );

  // A range change can leave the selected provider with nothing to show
  useEffect(() => {
    if (
      provider !== ALL_PROVIDERS &&
      availableProviders.length > 0 &&
      !availableProviders.includes(provider)
    ) {
      setProvider(ALL_PROVIDERS);
    }
  }, [availableProviders, provider]);

  // Everything below the picker sees one provider's records, or all of them
  const shown = useMemo(
    () =>
      provider === ALL_PROVIDERS
        ? history
        : history.filter((r) => r.provider_id === provider),
    [history, provider],
  );

  // Memoized computed data
  const chartData = useMemo(
    () => aggregateChartData(shown, granularity),
    [shown, granularity],
  );

  const providerBreakdowns = useMemo(
    () => computeProviderBreakdowns(shown),
    [shown],
  );

  const tokenSummary = useMemo(() => computeTokenTypeSummary(shown), [shown]);

  return (
    <div className="w-full h-full bg-gray-900 text-white p-6 flex flex-col gap-4 overflow-y-auto">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h1 className="text-lg font-bold">{t("dashboard.usageHistory")}</h1>
        {error && (
          <span className="text-xs text-red-400">{error}</span>
        )}
      </div>

      {/* Current quota, above the history and independent of its range: the
          range picker chooses a period to look back over, which is not a
          scope a live reading can be filtered by. */}
      <QuotaOverview
        highlightProviderId={provider === ALL_PROVIDERS ? null : provider}
      />

      {/* Controls Row */}
      <div className="flex flex-wrap items-center gap-4">
        {/* Time Range Selector */}
        <div className="flex items-center gap-2">
          <span className="text-xs text-white/60">{t("dashboard.timeRange")}:</span>
          <div className="flex gap-1">
            {(["day", "week", "month", "custom"] as TimeRange[]).map((range) => (
              <button
                key={range}
                onClick={() => setTimeRange(range)}
                className={`px-3 py-1 text-xs rounded transition-colors ${
                  timeRange === range
                    ? "bg-blue-600 text-white"
                    : "bg-white/10 text-white/70 hover:bg-white/20"
                }`}
              >
                {t(`dashboard.${range}`)}
              </button>
            ))}
          </div>
        </div>

        {/* Custom range: real inputs rather than a button that silently means
            "last 7 days". Shown only when Custom is the selected range. */}
        {timeRange === "custom" && (
          <div className="flex items-center gap-2 flex-wrap">
            <label className="text-xs text-white/60" htmlFor="range-start">
              {t("range.start")}
            </label>
            <input
              id="range-start"
              type="datetime-local"
              value={customRange.start}
              onChange={(e) =>
                setCustomRange((prev) => ({ ...prev, start: e.target.value }))
              }
              className="bg-white/10 text-white text-xs rounded px-2 py-1 border border-white/10"
            />
            <label className="text-xs text-white/60" htmlFor="range-end">
              {t("range.end")}
            </label>
            <input
              id="range-end"
              type="datetime-local"
              value={customRange.end}
              onChange={(e) =>
                setCustomRange((prev) => ({ ...prev, end: e.target.value }))
              }
              className="bg-white/10 text-white text-xs rounded px-2 py-1 border border-white/10"
            />
            {customRangeError(customRange) && (
              <span role="alert" className="text-xs text-red-400">
                {t("range.invalid")}
              </span>
            )}
          </div>
        )}

        {/* Provider Filter — one program at a time, or everything together */}
        {availableProviders.length > 1 && (
          <div className="flex items-center gap-2">
            <span className="text-xs text-white/60">{t("dashboard.provider")}:</span>
            <div className="flex gap-1">
              {[ALL_PROVIDERS, ...availableProviders].map((id) => (
                <button
                  key={id}
                  onClick={() => setProvider(id)}
                  className={`px-3 py-1 text-xs rounded transition-colors ${
                    provider === id
                      ? "bg-blue-600 text-white"
                      : "bg-white/10 text-white/70 hover:bg-white/20"
                  }`}
                >
                  {id === ALL_PROVIDERS ? t("dashboard.allProviders") : providerLabel(t, id)}
                </button>
              ))}
            </div>
          </div>
        )}

        {/* Granularity Picker */}
        <div className="flex items-center gap-2">
          <span className="text-xs text-white/60">{t("granularity.daily").split(" ")[0]}:</span>
          <select
            value={granularity}
            onChange={(e) => setGranularity(e.target.value as Granularity)}
            className="bg-white/10 text-white text-xs rounded px-2 py-1 border border-white/20 focus:outline-none focus:border-blue-500"
          >
            {(["hourly", "daily", "weekly", "monthly"] as Granularity[]).map((g) => (
              <option key={g} value={g} className="bg-gray-800 text-white">
                {t(`granularity.${g}`)}
              </option>
            ))}
          </select>
        </div>
      </div>

      {/* Loading State */}
      {historyLoading && (
        <div className="flex items-center justify-center py-8">
          <span className="text-sm text-white/50">{t("status.loading")}</span>
        </div>
      )}

      {/* Usage Chart */}
      {!historyLoading && (
        <div className="bg-white/5 rounded-lg p-4">
          <h2 className="text-sm font-semibold mb-3">{t("dashboard.usageHistory")}</h2>
          {chartData.length > 0 ? (
            <ResponsiveContainer width="100%" height={220}>
              <LineChart data={chartData}>
                <CartesianGrid strokeDasharray="3 3" stroke="rgba(255,255,255,0.1)" />
                <XAxis
                  dataKey="label"
                  tick={{ fill: "rgba(255,255,255,0.6)", fontSize: 10 }}
                  stroke="rgba(255,255,255,0.2)"
                />
                <YAxis
                  tick={{ fill: "rgba(255,255,255,0.6)", fontSize: 10 }}
                  stroke="rgba(255,255,255,0.2)"
                  tickFormatter={(value: number) => formatTokenCount(value)}
                />
                <Tooltip
                  contentStyle={{
                    backgroundColor: "#1f2937",
                    border: "1px solid rgba(255,255,255,0.2)",
                    borderRadius: "8px",
                    color: "#fff",
                  }}
                  labelFormatter={(_label: string, payload: Array<{ payload?: ChartDataPoint }>) => {
                    const point = payload?.[0]?.payload;
                    return point ? formatBangkokTime(point.timestamp) : _label;
                  }}
                  formatter={(value: number, name: string) => [
                    formatTokenCount(value),
                    name,
                  ]}
                />
                <Legend
                  wrapperStyle={{ fontSize: "11px", color: "rgba(255,255,255,0.7)" }}
                />
                <Line
                  type="monotone"
                  dataKey="inputTokens"
                  name={t("tokens.input")}
                  stroke="#60a5fa"
                  strokeWidth={2}
                  dot={false}
                />
                <Line
                  type="monotone"
                  dataKey="outputTokens"
                  name={t("tokens.output")}
                  stroke="#34d399"
                  strokeWidth={2}
                  dot={false}
                />
                <Line
                  type="monotone"
                  dataKey="totalTokens"
                  name={t("tokens.total")}
                  stroke="#fbbf24"
                  strokeWidth={2}
                  dot={false}
                />
              </LineChart>
            </ResponsiveContainer>
          ) : (
            <div className="h-[220px] flex items-center justify-center">
              <span className="text-sm text-white/40">{t("status.notAvailable")}</span>
            </div>
          )}
        </div>
      )}

      {/* Bottom Section: Provider Breakdown + Token Type Summary */}
      {!historyLoading && (
        <div className="grid grid-cols-2 gap-4">
          {/* Per-Provider Breakdown */}
          <div className="bg-white/5 rounded-lg p-4">
            <h2 className="text-sm font-semibold mb-3">{t("tokens.total")} — Provider</h2>
            {providerBreakdowns.length > 0 ? (
              <div className="flex flex-col gap-2 max-h-[200px] overflow-y-auto">
                {providerBreakdowns.map((breakdown) => (
                  <ProviderBreakdownRow
                    key={`${breakdown.providerId}-${breakdown.model}`}
                    breakdown={breakdown}
                  />
                ))}
              </div>
            ) : (
              <span className="text-xs text-white/40">{t("status.notAvailable")}</span>
            )}
          </div>

          {/* Token Type Breakdown */}
          <div className="bg-white/5 rounded-lg p-4">
            <h2 className="text-sm font-semibold mb-3">{t("tokens.total")} — Type</h2>
            <TokenTypePanel summary={tokenSummary} />
          </div>
        </div>
      )}
    </div>
  );
}

// ─── Subcomponents ──────────────────────────────────────────────────────────────

interface ProviderBreakdownRowProps {
  breakdown: ProviderBreakdown;
}

function ProviderBreakdownRow({ breakdown }: ProviderBreakdownRowProps) {
  const { t } = useTranslation();

  return (
    <div className="flex flex-col gap-0.5 py-1 border-b border-white/5 last:border-b-0">
      <div className="flex items-center justify-between">
        <span className="text-xs text-white/90 font-medium">
          {providerLabel(t, breakdown.providerId)}
        </span>
        <span className="text-[10px] text-white/50">{breakdown.model}</span>
      </div>
      <div className="flex items-center gap-3 text-[10px] text-white/60">
        <span>{t("tokens.input")}: {formatTokenCount(breakdown.inputTokens)}</span>
        <span>{t("tokens.output")}: {formatTokenCount(breakdown.outputTokens)}</span>
        <span>{t("tokens.total")}: {formatTokenCount(breakdown.totalTokens)}</span>
      </div>
    </div>
  );
}

interface TokenTypePanelProps {
  summary: TokenTypeSummary;
}

function TokenTypePanel({ summary }: TokenTypePanelProps) {
  const { t } = useTranslation();

  const items = [
    { label: t("tokens.input"), value: summary.input, color: "bg-blue-500" },
    { label: t("tokens.output"), value: summary.output, color: "bg-green-500" },
    { label: t("tokens.reasoning"), value: summary.reasoning, color: "bg-purple-500" },
    { label: t("tokens.cachedInput"), value: summary.cached, color: "bg-cyan-500" },
  ];

  // Unavailable values are excluded from the scale as well as the bars: a zero
  // would silently make every other bar look larger by comparison.
  const maxValue = Math.max(
    ...items.map((i) => i.value).filter((v): v is number => v !== null),
    1,
  );

  return (
    <div className="flex flex-col gap-2">
      {items.map((item) => (
        <div key={item.label} className="flex flex-col gap-0.5">
          <div className="flex items-center justify-between">
            <span className="text-[11px] text-white/70">{item.label}</span>
            <span
              className={`text-[11px] font-mono ${
                item.value === null ? "text-white/40 italic" : "text-white/90"
              }`}
              title={item.value === null ? t("tokens.unavailableWhy") : undefined}
            >
              {item.value === null ? t("tokens.unavailable") : formatTokenCount(item.value)}
            </span>
          </div>
          <div className="h-1.5 bg-white/10 rounded-full overflow-hidden">
            {item.value !== null && (
              <div
                className={`h-full ${item.color} rounded-full transition-all`}
                style={{ width: `${(item.value / maxValue) * 100}%` }}
              />
            )}
          </div>
        </div>
      ))}

      {/* Total */}
      <div className="flex items-center justify-between border-t border-white/10 pt-2 mt-1">
        <span className="text-xs text-white/90 font-semibold">{t("tokens.total")}</span>
        <span className="text-xs text-white/90 font-mono font-semibold">
          {formatTokenCount(summary.total)}
        </span>
      </div>
    </div>
  );
}
