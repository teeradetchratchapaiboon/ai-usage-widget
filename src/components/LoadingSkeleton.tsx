/**
 * LoadingSkeleton - Placeholder UI shown while data is loading.
 *
 * Mimics the compact widget layout with animated pulse bars.
 */

export function LoadingSkeleton() {
  return (
    <div className="flex flex-col gap-3 p-4 animate-pulse">
      {/* Title bar skeleton */}
      <div className="flex items-center justify-between">
        <div className="h-3 w-24 bg-white/20 rounded" />
        <div className="h-3 w-16 bg-white/20 rounded" />
      </div>

      {/* Provider rows skeleton */}
      <div className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <div className="w-2 h-2 rounded-full bg-white/20" />
          <div className="h-3 w-20 bg-white/20 rounded" />
          <div className="flex-1 h-2 bg-white/20 rounded-full" />
          <div className="h-3 w-12 bg-white/20 rounded" />
        </div>
        <div className="flex items-center gap-2">
          <div className="w-2 h-2 rounded-full bg-white/20" />
          <div className="h-3 w-20 bg-white/20 rounded" />
          <div className="flex-1 h-2 bg-white/20 rounded-full" />
          <div className="h-3 w-12 bg-white/20 rounded" />
        </div>
      </div>

      {/* Footer skeleton */}
      <div className="h-3 w-32 bg-white/20 rounded mt-auto" />
    </div>
  );
}
