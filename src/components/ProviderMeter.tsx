/**
 * ProviderMeter - Usage bar + percentage display for one provider.
 *
 * Shows a horizontal progress bar representing token usage or quota percentage
 * along with the formatted numeric value.
 */

interface ProviderMeterProps {
  /** Label for the meter (e.g., "Fast Hours") */
  label: string;
  /** Current percentage (0-100+, excess can exceed 100) */
  percentage: number | null;
  /** Formatted value text to display (e.g., "75.0%") */
  valueText: string;
}

export function ProviderMeter({ label, percentage, valueText }: ProviderMeterProps) {
  // Determine bar color based on percentage thresholds
  const getBarColor = (pct: number): string => {
    if (pct >= 90) return "bg-red-400";
    if (pct >= 75) return "bg-yellow-400";
    return "bg-blue-400";
  };

  // Clamp to 100 for visual display (bar width) but show actual value as text
  const clampedPct = percentage !== null ? Math.min(percentage, 100) : 0;

  return (
    <div className="flex items-center gap-2">
      <span className="text-[10px] text-white/70 w-14 truncate">{label}</span>
      <div className="flex-1 h-1.5 bg-white/10 rounded-full overflow-hidden">
        {percentage !== null && (
          <div
            className={`h-full rounded-full transition-all duration-300 ${getBarColor(percentage)}`}
            style={{ width: `${clampedPct}%` }}
          />
        )}
      </div>
      <span className="text-[10px] text-white/80 w-10 text-right">{valueText}</span>
    </div>
  );
}
