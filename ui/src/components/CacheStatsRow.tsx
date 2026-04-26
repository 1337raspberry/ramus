import { useCallback, useEffect, useState } from "react";

export function ImageCacheRow() {
  const [stats, setStats] = useState<{
    entryCount: number;
    totalSizeBytes: number;
    pinnedCount: number;
    pinnedSizeBytes: number;
  } | null>(null);
  const refresh = useCallback(() => {
    import("../lib/commands").then(({ getImageCacheStats }) =>
      getImageCacheStats()
        .then(setStats)
        .catch(() => {}),
    );
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);
  const mb = stats ? (stats.totalSizeBytes / 1_048_576).toFixed(1) : "—";
  const count = stats?.entryCount ?? 0;
  const pinnedMb = stats ? (stats.pinnedSizeBytes / 1_048_576).toFixed(1) : "—";
  const pinnedCount = stats?.pinnedCount ?? 0;
  return (
    <div className="settings-row">
      <span>
        {count} images, {mb} MB
        {pinnedCount > 0 && (
          <>
            {" "}
            <span style={{ opacity: 0.6 }}>
              ({pinnedCount} pinned, {pinnedMb} MB — kept for offline downloads)
            </span>
          </>
        )}
      </span>
      <button
        className="settings-btn"
        onClick={() => {
          import("../lib/commands").then(({ flushImageCache }) =>
            flushImageCache()
              .then(refresh)
              .catch(() => {}),
          );
        }}
      >
        Flush
      </button>
    </div>
  );
}

export function AudioCacheRow() {
  const [stats, setStats] = useState<{ entryCount: number; totalSizeBytes: number } | null>(null);
  const refresh = useCallback(() => {
    import("../lib/commands").then(({ getAudioCacheStats }) =>
      getAudioCacheStats()
        .then(setStats)
        .catch(() => {}),
    );
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);
  const mb = stats ? (stats.totalSizeBytes / 1_048_576).toFixed(1) : "—";
  const count = stats?.entryCount ?? 0;
  return (
    <div className="settings-row">
      <span>
        {count} tracks, {mb} MB
      </span>
      <button
        className="settings-btn"
        onClick={() => {
          import("../lib/commands").then(({ clearAudioCache }) =>
            clearAudioCache()
              .then(refresh)
              .catch(() => {}),
          );
        }}
      >
        Clear
      </button>
    </div>
  );
}
