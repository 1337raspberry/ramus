import { useCallback, useEffect, useState } from "react";

import { useDownloadsStore } from "../stores/downloadsStore";
import { formatBytes } from "../lib/format";
import type {
  DownloadedAlbumSummary,
  DownloadedTrackSummary,
  InProgressDownload,
} from "../lib/types";
import { useArtUrl } from "../lib/useArtUrl";
import { ART_SIZE } from "../lib/commands";

interface Props {
  onDismiss: () => void;
}

export default function DownloadsPanel({ onDismiss }: Props) {
  const overview = useDownloadsStore((s) => s.overview);
  const liveProgress = useDownloadsStore((s) => s.liveProgress);
  const refresh = useDownloadsStore((s) => s.refresh);
  const cancelAll = useDownloadsStore((s) => s.cancelAll);
  const remove = useDownloadsStore((s) => s.remove);
  const removeAlbum = useDownloadsStore((s) => s.removeAlbum);
  const clearAll = useDownloadsStore((s) => s.clearAll);
  const startStarredTracks = useDownloadsStore((s) => s.startStarredTracks);
  const startStarredAlbums = useDownloadsStore((s) => s.startStarredAlbums);
  const estimateStarredTracks = useDownloadsStore((s) => s.estimateStarredTracks);
  const estimateStarredAlbums = useDownloadsStore((s) => s.estimateStarredAlbums);

  const [starredTracksEst, setStarredTracksEst] = useState<number | null>(null);
  const [starredAlbumsEst, setStarredAlbumsEst] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);

  useEffect(() => {
    refresh();
    estimateStarredTracks()
      .then(setStarredTracksEst)
      .catch(() => {});
    estimateStarredAlbums()
      .then(setStarredAlbumsEst)
      .catch(() => {});
  }, [refresh, estimateStarredTracks, estimateStarredAlbums]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        if (confirmClear) {
          setConfirmClear(false);
        } else {
          onDismiss();
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onDismiss, confirmClear]);

  const handleBackdrop = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onDismiss();
  };

  const handleStarredTracks = useCallback(async () => {
    try {
      const n = await startStarredTracks();
      if (n === 0) setError("No downloadable starred tracks found.");
    } catch (e) {
      setError(String(e));
    }
  }, [startStarredTracks]);

  const handleStarredAlbums = useCallback(async () => {
    try {
      const n = await startStarredAlbums();
      if (n === 0) setError("No downloadable starred albums found.");
    } catch (e) {
      setError(String(e));
    }
  }, [startStarredAlbums]);

  const handleClearAll = useCallback(async () => {
    try {
      await clearAll();
      setConfirmClear(false);
    } catch (e) {
      setError(String(e));
    }
  }, [clearAll]);

  const albumsCount = overview?.albums.length ?? 0;
  const tracksCount = overview?.orphanTracks.length ?? 0;
  const totalBytes = overview?.totalBytes ?? 0;
  const queueLen = overview?.queueLen ?? 0;
  const inProgress = overview?.inProgress ?? null;

  return (
    <div className="settings-backdrop" onClick={handleBackdrop}>
      <div className="settings-panel downloads-panel glass">
        <div className="settings-header">
          <h2>Downloads</h2>
          <button className="settings-close" onClick={onDismiss}>
            x
          </button>
        </div>

        <div className="settings-body downloads-body">
          {error && (
            <div className="settings-error" onClick={() => setError(null)}>
              {error}
            </div>
          )}

          <InProgressSection
            inProgress={inProgress}
            liveProgress={liveProgress}
            queueLen={queueLen}
            onCancelAll={cancelAll}
          />

          <section className="downloads-section">
            <h3 className="downloads-section-title">Storage</h3>
            <div className="downloads-storage-row">
              <div className="downloads-storage-primary">{formatBytes(totalBytes)} used</div>
              <div className="downloads-storage-secondary">
                {albumsCount} album{albumsCount === 1 ? "" : "s"} · {tracksCount} individual track
                {tracksCount === 1 ? "" : "s"}
              </div>
            </div>
            {totalBytes > 0 && !confirmClear && (
              <button
                className="settings-btn settings-btn-danger"
                onClick={() => setConfirmClear(true)}
              >
                Remove all downloads
              </button>
            )}
            {confirmClear && (
              <div className="downloads-confirm-row">
                <span>Remove every downloaded file?</span>
                <button className="settings-btn settings-btn-danger" onClick={handleClearAll}>
                  Remove all
                </button>
                <button className="settings-btn" onClick={() => setConfirmClear(false)}>
                  Cancel
                </button>
              </div>
            )}
          </section>

          <section className="downloads-section">
            <h3 className="downloads-section-title">Bulk downloads</h3>
            <button className="settings-btn" onClick={handleStarredTracks}>
              Download all starred tracks
              {starredTracksEst !== null && (
                <span className="downloads-btn-detail"> (~{formatBytes(starredTracksEst)})</span>
              )}
            </button>
            <button className="settings-btn" onClick={handleStarredAlbums}>
              Download all starred albums
              {starredAlbumsEst !== null && (
                <span className="downloads-btn-detail"> (~{formatBytes(starredAlbumsEst)})</span>
              )}
            </button>
          </section>

          <section className="downloads-section">
            <h3 className="downloads-section-title">
              Cached albums {albumsCount > 0 && <span>({albumsCount})</span>}
            </h3>
            {albumsCount === 0 ? (
              <div className="downloads-empty">No albums downloaded yet.</div>
            ) : (
              <ul className="downloads-list">
                {overview?.albums.map((a) => (
                  <AlbumRow key={a.ratingKey} album={a} onRemove={() => removeAlbum(a.ratingKey)} />
                ))}
              </ul>
            )}
          </section>

          <section className="downloads-section">
            <h3 className="downloads-section-title">
              Individual tracks {tracksCount > 0 && <span>({tracksCount})</span>}
            </h3>
            {tracksCount === 0 ? (
              <div className="downloads-empty">No individual tracks.</div>
            ) : (
              <ul className="downloads-list">
                {overview?.orphanTracks.map((t) => (
                  <TrackRow key={t.ratingKey} track={t} onRemove={() => remove(t.ratingKey)} />
                ))}
              </ul>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}

function InProgressSection({
  inProgress,
  liveProgress,
  queueLen,
  onCancelAll,
}: {
  inProgress: InProgressDownload | null;
  liveProgress: { bytesWritten: number; totalBytes: number | null } | null;
  queueLen: number;
  onCancelAll: () => Promise<void>;
}) {
  if (!inProgress && queueLen === 0) return null;

  // Prefer the live-progress bytes from the event stream over the overview
  // snapshot, which only updates on downloads-changed.
  const bytes = liveProgress?.bytesWritten ?? inProgress?.bytesWritten ?? 0;
  const total = liveProgress?.totalBytes ?? inProgress?.totalBytes ?? null;

  const art = useArtUrl(inProgress?.thumb ?? null, ART_SIZE.SMALL);

  const pct = total && total > 0 ? Math.min(100, Math.round((bytes / total) * 100)) : null;

  return (
    <section className="downloads-section">
      <div className="downloads-inprogress-header">
        <h3 className="downloads-section-title">
          In progress{queueLen > 0 && <span> ({queueLen} queued)</span>}
        </h3>
        <button className="downloads-text-link" onClick={onCancelAll}>
          Cancel all
        </button>
      </div>
      {inProgress && (
        <div className="downloads-inflight-card">
          <div className="downloads-item-art downloads-inflight-art">
            {art.artSrc && !art.artErr && <img src={art.artSrc} alt="" />}
          </div>
          <div className="downloads-inflight-text">
            <div className="downloads-inflight-title">{inProgress.title}</div>
            <div className="downloads-inflight-sub">
              {inProgress.artistName}
              {inProgress.albumTitle ? ` · ${inProgress.albumTitle}` : ""}
            </div>
            <div className="sync-progress-bar-bg">
              <div
                className="sync-progress-bar-fill"
                style={{
                  width: pct !== null ? `${pct}%` : "5%",
                }}
              />
            </div>
            <div className="downloads-inflight-meta">
              {formatBytes(bytes)}
              {total ? ` / ${formatBytes(total)}` : ""}
              {pct !== null ? ` · ${pct}%` : ""}
            </div>
          </div>
        </div>
      )}
      {queueLen > 0 && !inProgress && (
        <div className="downloads-empty">Waiting for worker… ({queueLen} queued)</div>
      )}
    </section>
  );
}

function AlbumRow({ album, onRemove }: { album: DownloadedAlbumSummary; onRemove: () => void }) {
  const { artSrc: art } = useArtUrl(album.thumb, ART_SIZE.SMALL);
  const partial = album.downloaded < album.total;
  return (
    <li className="downloads-item-row">
      <div className="downloads-item-art">{art && <img src={art} alt="" />}</div>
      <div className="downloads-item-text">
        <div className="downloads-item-title">{album.title}</div>
        <div className="downloads-item-sub">
          {album.artistName} · {formatBytes(album.sizeBytes)}
          {partial && (
            <>
              {" "}
              ·{" "}
              <span className="downloads-partial">
                {album.downloaded}/{album.total} tracks
              </span>
            </>
          )}
        </div>
      </div>
      <button className="downloads-row-action" onClick={onRemove} title="Remove downloaded files">
        x
      </button>
    </li>
  );
}

function TrackRow({ track, onRemove }: { track: DownloadedTrackSummary; onRemove: () => void }) {
  const { artSrc: art } = useArtUrl(track.thumb, ART_SIZE.SMALL);
  return (
    <li className="downloads-item-row">
      <div className="downloads-item-art">{art && <img src={art} alt="" />}</div>
      <div className="downloads-item-text">
        <div className="downloads-item-title">{track.title}</div>
        <div className="downloads-item-sub">
          {track.artistName} · {track.albumTitle} · {formatBytes(track.sizeBytes)}
        </div>
      </div>
      <button className="downloads-row-action" onClick={onRemove} title="Remove download">
        x
      </button>
    </li>
  );
}
