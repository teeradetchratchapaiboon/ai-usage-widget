/**
 * CompactWidget - Main 340×200px compact widget view.
 *
 * Displays token usage summary per provider with auto-refresh every 10 seconds.
 * Uses Windows 11 glass (Acrylic) visual effect via backdrop-filter.
 */

import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store";
import { formatTokenCount, formatRelative } from "../lib/format";
import { StatusDot } from "./StatusDot";
import { ProviderMeter } from "./ProviderMeter";
import { LoadingSkeleton } from "./LoadingSkeleton";

export function CompactWidget() {
  const { t } = useTranslation();
  const { usage, providers, isLoading, fetchUsage, fetchProviderStatus } = useAppStore();

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
      <div className="widget-glass w-[340px] h-[200px] rounded-lg p-3 flex flex-col">
        <LoadingSkeleton />
      </div>
    );
  }

  return (
    <div className="widget-glass w-[340px] h-[200px] rounded-lg p-3 flex flex-col gap-2 overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h1 className="text-xs font-semibold text-white/90">
          {t("widget.title")}
        </h1>
        <span className="text-[10px] text-white/50">
          {t("tokens.total")}: {formatTokenCount(usage?.total_tokens_today ?? null)}
        </span>
      </div>

      {/* Provider list */}
      <div className="flex-1 flex flex-col gap-2 overflow-y-auto">
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
            />
          );
        })}

        {providers.length === 0 && !isLoading && (
          <div className="flex-1 flex items-center justify-center">
            <span className="text-xs text-white/50">{t("status.notAvailable")}</span>
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="flex items-center justify-between border-t border-white/10 pt-1">
        <span className="text-[10px] text-white/40">
          {usage?.last_updated
            ? `${t("time.lastUpdated")}: ${formatRelative(usage.last_updated)}`
            : ""}
        </span>
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
}

function ProviderRow({
  providerId,
  displayName,
  isAvailable,
  totalTokensToday,
  lastActivity,
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

  // Calculate a rough percentage for the usage meter (based on daily tokens)
  // Since we don't have quota limits from the IPC data, show raw token count
  const tokenText = formatTokenCount(totalTokensToday);

  return (
    <div className="flex flex-col gap-1 py-1">
      <div className="flex items-center gap-2">
        <StatusDot available={true} />
        <span className="text-[11px] text-white/90 flex-1">{name}</span>
        <span className="text-[10px] text-white/70">{tokenText}</span>
      </div>
      {totalTokensToday !== null && (
        <ProviderMeter
          label={t("tokens.total")}
          percentage={null}
          valueText={tokenText}
        />
      )}
      {lastActivity && (
        <span className="text-[9px] text-white/40 pl-4">
          {formatRelative(lastActivity)}
        </span>
      )}
    </div>
  );
}
