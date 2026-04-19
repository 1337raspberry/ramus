import { useCallback, useEffect, useState } from "react";

import { useDownloadsStore } from "../stores/downloadsStore";
import { formatBytes } from "../lib/format";
import type { DownloadedAlbumSummary, DownloadedTrackSummary } from "../lib/types";
import { useArtUrl } from "../lib/useArtUrl";
import { ART_SIZE } from "../lib/commands";

interface Props {
  onDismiss: () => void;
}

export default function DownloadsPanel({ onDismiss }: Props) {
  const overview = useDownloadsStore((s) => s.overview);
  const trackState = useDownloadsStore((s) => s.trackState);
  const refresh = useDownloadsStore((s) => s.refresh);
  const cancel = useDownloadsStore((s) => s.cancel);
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
            trackState={trackState}
            queue={overview?.queue ?? []}
            onCancel={cancel}
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
  trackState,
  queue,
  onCancel,
  onCancelAll,
}: {
  trackState: Record<
    string,
    { phase: string; bytesWritten: number; totalBytes: number | null; error: string | null }
  >;
  queue: string[];
  onCancel: (ratingKey: string) => Promise<void>;
  onCancelAll: () => Promise<void>;
}) {
  const entries = Object.entries(trackState).filter(
    ([, s]) => s.phase === "downloading" || s.phase === "queued",
  );
  if (entries.length === 0) return null;

  return (
    <section className="downloads-section">
      <div className="downloads-inprogress-header">
        <h3 className="downloads-section-title">In progress ({entries.length})</h3>
        <button className="downloads-text-link" onClick={onCancelAll}>
          Cancel all
        </button>
      </div>
      <ul className="downloads-list">
        {entries
          .sort(([a], [b]) => {
            // In-flight first, then queue order.
            const ai = queue.indexOf(a);
            const bi = queue.indexOf(b);
            if (ai < 0 && bi < 0) return 0;
            if (ai < 0) return -1;
            if (bi < 0) return 1;
            return ai - bi;
          })
          .map(([ratingKey, st]) => (
            <li key={ratingKey} className="downloads-inflight-row">
              <div className="downloads-inflight-title">{ratingKey}</div>
              <div className="downloads-inflight-meta">
                {st.phase === "downloading" ? (
                  <>
                    {formatBytes(st.bytesWritten)}
                    {st.totalBytes ? ` / ${formatBytes(st.totalBytes)}` : ""}
                  </>
                ) : (
                  <span className="downloads-queued">Queued</span>
                )}
              </div>
              <div className="sync-progress-bar-bg">
                <div
                  className="sync-progress-bar-fill"
                  style={{
                    width:
                      st.totalBytes && st.totalBytes > 0
                        ? `${Math.min(100, Math.round((st.bytesWritten / st.totalBytes) * 100))}%`
                        : st.phase === "downloading"
                          ? "5%"
                          : "0%",
                  }}
                />
              </div>
              <button
                className="downloads-row-action"
                onClick={() => onCancel(ratingKey)}
                title="Cancel"
              >
                x
              </button>
            </li>
          ))}
      </ul>
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
