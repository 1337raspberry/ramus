import { useCallback, useMemo } from "react";

import {
  useDownloadsStore,
  selectIsAlbumDownloaded,
  selectIsDownloaded,
  selectTrackDownloadState,
} from "../stores/downloadsStore";

/// "Download" / "Downloading 42%…" / "Remove Download" item for a single
/// track's (...) menu. Keep in sync with `AlbumDownloadMenuItem`.
export function TrackDownloadMenuItem({
  ratingKey,
  onDone,
}: {
  ratingKey: string;
  onDone?: () => void;
}) {
  const isDownloaded = useDownloadsStore(selectIsDownloaded(ratingKey));
  const state = useDownloadsStore(selectTrackDownloadState(ratingKey));
  const start = useDownloadsStore((s) => s.startTrackDownload);
  const cancel = useDownloadsStore((s) => s.cancel);
  const remove = useDownloadsStore((s) => s.remove);

  const phase = state?.phase;
  const label = useMemo(() => {
    if (phase === "downloading") {
      if (state?.totalBytes && state.totalBytes > 0) {
        const pct = Math.min(100, Math.round((state.bytesWritten / state.totalBytes) * 100));
        return `Downloading ${pct}%…`;
      }
      return "Downloading…";
    }
    if (phase === "queued") return "Queued";
    if (isDownloaded) return "Remove Download";
    return "Download";
  }, [phase, state?.bytesWritten, state?.totalBytes, isDownloaded]);

  const handleClick = useCallback(
    async (e: React.MouseEvent) => {
      e.stopPropagation();
      try {
        if (phase === "downloading" || phase === "queued") {
          await cancel(ratingKey);
        } else if (isDownloaded) {
          await remove(ratingKey);
        } else {
          await start(ratingKey);
        }
      } catch (err) {
        console.warn("download action failed", err);
      }
      onDone?.();
    },
    [phase, isDownloaded, ratingKey, start, cancel, remove, onDone],
  );

  return (
    <button onClick={handleClick} data-download-state={phase ?? (isDownloaded ? "done" : "idle")}>
      {label}
    </button>
  );
}

/// "Download" / "Remove Download" item for an album's (...) menu. Kicks
/// off per-track enqueuing on the backend; cancellation / progress are
/// tracked per-track.
export function AlbumDownloadMenuItem({
  albumRatingKey,
  onDone,
}: {
  albumRatingKey: string;
  onDone?: () => void;
}) {
  const isDownloaded = useDownloadsStore(selectIsAlbumDownloaded(albumRatingKey));
  const start = useDownloadsStore((s) => s.startAlbumDownload);
  const removeAlbum = useDownloadsStore((s) => s.removeAlbum);

  const handleClick = useCallback(
    async (e: React.MouseEvent) => {
      e.stopPropagation();
      try {
        if (isDownloaded) {
          await removeAlbum(albumRatingKey);
        } else {
          await start(albumRatingKey);
        }
      } catch (err) {
        console.warn("album download action failed", err);
      }
      onDone?.();
    },
    [isDownloaded, albumRatingKey, start, removeAlbum, onDone],
  );

  return <button onClick={handleClick}>{isDownloaded ? "Remove Download" : "Download"}</button>;
}
