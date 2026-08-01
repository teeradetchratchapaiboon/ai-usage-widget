/**
 * Current Quota Overview — remaining quota, on the Dashboard.
 *
 * Until now the live quota existed only in the widget, so opening the
 * dashboard (which hides the widget) meant losing sight of the one number that
 * decides whether you can keep working. This panel is deliberately independent
 * of the history below it: the historical range picker selects a period to
 * *look back over*, and applying it to a current reading would be meaningless.
 */

import { useTranslation } from "react-i18next";
import { useAppStore } from "../store";
import { formatCountdown, formatResetTime } from "../lib/format";
import { presentFreshness, freshnessBadgeClass, formatUpdatedAgo } from "../lib/freshness";
import { quotaWindows, remainingPct, type QuotaWindow } from "../lib/quota";
import type { ProviderStatus } from "../lib/ipc";
import { StatusDot } from "./StatusDot";

interface QuotaOverviewProps {
  /** Provider id the history filter has selected, for emphasis only. */
  highlightProviderId?: string | null;
}

export function QuotaOverview({ highlightProviderId }: QuotaOverviewProps) {
  const { t } = useTranslation();
  const { providers, providerStatusLoading, collectionLoading, triggerCollection } =
    useAppStore();

  return (
    <section className="mb-4" aria-label={t("quota.current")}>
      <div className="flex items-center justify-between gap-2 mb-2">
        <h2 className="text-sm font-semibold text-white/90">{t("quota.current")}</h2>
        <button
          type="button"
          onClick={() => void triggerCollection()}
          disabled={collectionLoading}
          // Says what it can actually do: re-read local files. It cannot ask
          // the provider for a newer quota, which is why an old reading can
          // survive a collection.
          title={t("collect.explain")}
          className="text-[11px] px-2 py-1 rounded bg-white/10 hover:bg-white/20 disabled:opacity-50 text-white/80 transition-colors"
        >
          {collectionLoading ? t("collect.collecting") : t("collect.now")}
        </button>
      </div>

      {providers.length === 0 ? (
        <p className="text-xs text-white/50">
          {providerStatusLoading ? t("collect.collecting") : t("status.notAvailable")}
        </p>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          {providers.map((provider) => (
            <ProviderQuotaCard
              key={provider.provider_id}
              provider={provider}
              highlighted={highlightProviderId === provider.provider_id}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function ProviderQuotaCard({
  provider,
  highlighted,
}: {
  provider: ProviderStatus;
  highlighted: boolean;
}) {
  const { t } = useTranslation();
  const windows = quotaWindows(provider);

  return (
    <article
      className={`rounded-lg border p-3 bg-white/5 transition-colors ${
        highlighted ? "border-blue-400/50" : "border-white/10"
      }`}
    >
      <header className="flex items-center gap-2 mb-2 min-w-0">
        <StatusDot available={provider.is_available} />
        <span className="text-xs font-medium text-white/90 truncate flex-1">
          {provider.display_name}
        </span>
        {!provider.is_available && (
          <span className="text-[10px] text-white/50">{t("status.notAvailable")}</span>
        )}
      </header>

      <div className="flex flex-col gap-2">
        {windows.map((window) => (
          <QuotaWindowRow key={window.labelKey} window={window} />
        ))}
      </div>
    </article>
  );
}

function QuotaWindowRow({ window }: { window: QuotaWindow }) {
  const { t } = useTranslation();

  const look = presentFreshness(window.freshness, window.ageSecs);
  // Expired means the window rolled over, so the stored figure is known to be
  // wrong rather than merely old — showing it would misinform.
  const left =
    window.usedPct === null || look.suppressesValue ? null : remainingPct(window.usedPct);
  const countdown = formatCountdown(window.resetsAt, { approximate: window.estimated });
  const updatedAgo = formatUpdatedAgo(window.ageSecs);

  const observedTooltip = window.observedAt
    ? `${t("freshness.observedAt")}: ${formatResetTime(window.observedAt)}`
    : t("freshness.ageUnknown");
  const resetTooltip = window.resetsAt
    ? [formatResetTime(window.resetsAt), window.estimated ? t("quota.estimated") : null]
        .filter(Boolean)
        .join(" · ")
    : undefined;

  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-baseline gap-2 min-w-0">
        <span className="text-[11px] text-white/70 w-20 shrink-0 truncate">
          {t(window.labelKey)}
        </span>
        <span
          className={`text-sm font-semibold tabular-nums ${look.valueClass} ${
            look.showsValueAsCurrent ? "text-white" : "text-white/60"
          }`}
        >
          {left === null ? "—" : `${left.toFixed(0)}%`}
        </span>
        <span
          title={observedTooltip}
          className={`text-[9px] px-1.5 py-0.5 rounded shrink-0 ${freshnessBadgeClass(
            window.freshness,
          )}`}
        >
          {look.badge}
        </span>
      </div>

      <div className="flex flex-wrap gap-x-3 gap-y-0.5 pl-[5.5rem] text-[10px]">
        {/* Age first: it decides whether the countdown means anything */}
        {updatedAgo && (
          <span title={observedTooltip} className={look.captionClass}>
            {updatedAgo}
          </span>
        )}
        {look.caption && !updatedAgo && (
          <span title={observedTooltip} className={look.captionClass}>
            {look.caption}
          </span>
        )}
        {look.caption && updatedAgo && look.caption !== updatedAgo && !look.showsValueAsCurrent && (
          <span title={observedTooltip} className={look.captionClass}>
            {look.caption}
          </span>
        )}
        {countdown && !look.suppressesValue && (
          <span title={resetTooltip} className="text-white/40">
            {t("quota.resetsAt")}: {countdown}
          </span>
        )}
      </div>
    </div>
  );
}
