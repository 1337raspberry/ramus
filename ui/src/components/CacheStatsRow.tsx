import { useCallback, useEffect, useState } from "react";
import {
  clearAudioCache,
  flushImageCache,
  getAudioCacheStats,
  getImageCacheStats,
} from "../lib/commands";
import { HelperText } from "./HelperText";

interface CacheRowProps {
  limitBytes: number;
  /// Called when the user changes the GB number input. Caller is
  /// responsible for clamping + persisting to settings.
  onLimitChange: (nextBytes: number) => void;
}

/// Adjustable upper bound (GB) on the GB number input. Differs between
/// audio (large library cache) and image (small per-art cache).
const AUDIO_MAX_GB = 50;
const IMAGE_MAX_GB = 10;
const BYTES_PER_GB = 1_073_741_824;

/// Convert a bytes value to a human GB string, rounded to one decimal.
function gbString(bytes: number) {
  return (bytes / BYTES_PER_GB).toFixed(1);
}

/// Clamp a user-entered GB value to the allowed range and convert back to bytes.
function clampGbToBytes(input: number, maxGb: number) {
  const gb = Math.max(0.1, Math.min(maxGb, input));
  return Math.round(gb * BYTES_PER_GB);
}

export function ImageCacheRow({ limitBytes, onLimitChange }: CacheRowProps) {
  const [stats, setStats] = useState<{
    entryCount: number;
    totalSizeBytes: number;
    pinnedCount: number;
    pinnedSizeBytes: number;
  } | null>(null);
  const refresh = useCallback(() => {
    getImageCacheStats()
      .then(setStats)
      .catch(() => {});
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);
  const mb = stats ? (stats.totalSizeBytes / 1_048_576).toFixed(1) : "—";
  const count = stats?.entryCount ?? 0;
  const pinnedMb = stats ? (stats.pinnedSizeBytes / 1_048_576).toFixed(1) : "—";
  const pinnedCount = stats?.pinnedCount ?? 0;
  return (
    <>
      <label className="settings-row settings-row-multi">
        <span>Image cache limit (GB)</span>
        <div className="settings-controls-pair">
          <input
            type="number"
            className="settings-number-input"
            min={0.1}
            max={IMAGE_MAX_GB}
            step={0.1}
            value={gbString(limitBytes)}
            onChange={(e) => onLimitChange(clampGbToBytes(Number(e.target.value), IMAGE_MAX_GB))}
            onBlur={(e) => onLimitChange(clampGbToBytes(Number(e.target.value), IMAGE_MAX_GB))}
          />
          <button
            className="settings-btn"
            onClick={() => {
              flushImageCache()
                .then(refresh)
                .catch(() => {});
            }}
          >
            Clear
          </button>
        </div>
      </label>
      <HelperText>
        {count} images, {mb} MB cached
        {pinnedCount > 0 && (
          <>
            {" "}
            ({pinnedCount} pinned, {pinnedMb} MB — kept for offline downloads)
          </>
        )}
      </HelperText>
    </>
  );
}

export function AudioCacheRow({ limitBytes, onLimitChange }: CacheRowProps) {
  const [stats, setStats] = useState<{
    entryCount: number;
    totalSizeBytes: number;
  } | null>(null);
  const refresh = useCallback(() => {
    getAudioCacheStats()
      .then(setStats)
      .catch(() => {});
  }, []);
  useEffect(() => {
    refresh();
  }, [refresh]);
  const mb = stats ? (stats.totalSizeBytes / 1_048_576).toFixed(1) : "—";
  const count = stats?.entryCount ?? 0;
  return (
    <>
      <label className="settings-row settings-row-multi">
        <span>Audio cache limit (GB)</span>
        <div className="settings-controls-pair">
          <input
            type="number"
            className="settings-number-input"
            min={0.1}
            max={AUDIO_MAX_GB}
            step={0.1}
            value={gbString(limitBytes)}
            onChange={(e) => onLimitChange(clampGbToBytes(Number(e.target.value), AUDIO_MAX_GB))}
            onBlur={(e) => onLimitChange(clampGbToBytes(Number(e.target.value), AUDIO_MAX_GB))}
          />
          <button
            className="settings-btn"
            onClick={() => {
              clearAudioCache()
                .then(refresh)
                .catch(() => {});
            }}
          >
            Clear
          </button>
        </div>
      </label>
      <HelperText>
        {count} tracks, {mb} MB cached
      </HelperText>
    </>
  );
}
