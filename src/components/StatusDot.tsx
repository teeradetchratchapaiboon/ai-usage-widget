/**
 * StatusDot - Small colored dot indicating provider availability.
 *
 * Green = available, Gray = unavailable.
 */

interface StatusDotProps {
  available: boolean;
}

export function StatusDot({ available }: StatusDotProps) {
  return (
    <span
      className={`inline-block w-2 h-2 rounded-full ${
        available ? "bg-green-400" : "bg-gray-500"
      }`}
      aria-label={available ? "Available" : "Unavailable"}
    />
  );
}
