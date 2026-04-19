import { useCallback } from "react";

import {
  useDownloadsStore,
  selectIsAlbumDownloaded,
  selectIsDownloaded,
  selectTrackPhase,
} from "../stores/downloadsStore";

/// "Download" / "Downloading…" / "Queued" / "Remove Download" item for a
/// single track's (...) menu.
export function TrackDownloadMenuItem({
  ratingKey,
  onDone,
}: {
  ratingKey: string;
  onDone?: () => void;
}) {
  const isDownloaded = useDownloadsStore(selectIsDownloaded(ratingKey));
  const phase = useDownloadsStore(selectTrackPhase(ratingKey));
  const start = useDownloadsStore((s) => s.startTrackDownload);
  const cancel = useDownloadsStore((s) => s.cancel);
  const remove = useDownloadsStore((s) => s.remove);

  const label =
    phase === "downloading"
      ? "Downloading…"
      : phase === "queued"
        ? "Queued"
        : isDownloaded
          ? "Remove Download"
          : "Download";

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

/// "Download" / "Remove Download" item for an album's (...) menu.
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
