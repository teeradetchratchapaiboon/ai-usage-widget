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
  reasoning: number;
  cached: number;
  total: number;
}

// ─── Helpers ────────────────────────────────────────────────────────────────────

/** Picker value that applies no provider filter. */
const ALL_PROVIDERS = "__all__";

/** Translated provider name, falling back to the raw id. */
function providerLabel(t: (key: string) => string, providerId: string): string {
  if (providerId === "codex") return t("provider.codex");
  if (providerId === "claude") return t("provider.claude");
  return providerId;
}

function getTimeRangeStart(range: TimeRange): string {
  const now = new Date();
  switch (range) {
    case "day":
      return new Date(now.getTime() - 24 * 60 * 60 * 1000).toISOString();
    case "week":
      return new Date(now.getTime() - 7 * 24 * 60 * 60 * 1000).toISOString();
    case "month":
      return new Date(now.getTime() - 30 * 24 * 60 * 60 * 1000).toISOString();
    case "custom":
      // Default to 7 days for custom (user can refine later)
      return new Date(now.getTime() - 7 * 24 * 60 * 60 * 1000).toISOString();
  }
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

  // Reasoning and cached are not directly available in UsageRecord from IPC,
  // but we estimate reasoning = total - input - output (if positive)
  const reasoning = Math.max(0, total - input - output);
  // Cached is not available in the aggregated history endpoint
  const cached = 0;

  return { input, output, reasoning, cached, total };
}

// ─── Component ──────────────────────────────────────────────────────────────────

export function Dashboard() {
  const { t } = useTranslation();
  const { history, isLoading, error, fetchHistory } = useAppStore();

  const [timeRange, setTimeRange] = useState<TimeRange>("week");
  const [granularity, setGranularity] = useState<Granularity>("daily");
  const [provider, setProvider] = useState<string>(ALL_PROVIDERS);

  // Fetch history when time range or granularity changes
  const loadHistory = useCallback(() => {
    const start = getTimeRangeStart(timeRange);
    const end = new Date().toISOString();
    fetchHistory(start, end, granularity);
  }, [timeRange, granularity, fetchHistory]);

  useEffect(() => {
    loadHistory();
  }, [loadHistory]);

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
      {isLoading && (
        <div className="flex items-center justify-center py-8">
          <span className="text-sm text-white/50">{t("status.loading")}</span>
        </div>
      )}

      {/* Usage Chart */}
      {!isLoading && (
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
      {!isLoading && (
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

  const maxValue = Math.max(...items.map((i) => i.value), 1);

  return (
    <div className="flex flex-col gap-2">
      {items.map((item) => (
        <div key={item.label} className="flex flex-col gap-0.5">
          <div className="flex items-center justify-between">
            <span className="text-[11px] text-white/70">{item.label}</span>
            <span className="text-[11px] text-white/90 font-mono">
              {formatTokenCount(item.value)}
            </span>
          </div>
          <div className="h-1.5 bg-white/10 rounded-full overflow-hidden">
            <div
              className={`h-full ${item.color} rounded-full transition-all`}
              style={{ width: `${(item.value / maxValue) * 100}%` }}
            />
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
