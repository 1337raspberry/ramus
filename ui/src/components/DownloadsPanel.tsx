import { useCallback, useEffect, useState } from "react";

import { useDownloadsStore } from "../stores/downloadsStore";
import { useSettingsStore } from "../stores/settingsStore";
import { formatBytes } from "../lib/format";
import type {
  BookmarkDownloadEstimate,
  DownloadedAlbumSummary,
  DownloadedTrackSummary,
  DownloadQuality,
  InProgressDownload,
  Playlist,
  PlaylistDownloadEstimate,
} from "../lib/types";
import { filtersFromBookmark } from "../lib/bookmark";
import { describeFilters } from "../lib/filterDescribe";
import { useArtUrl } from "../lib/useArtUrl";
import { ART_SIZE, getPlaylists } from "../lib/commands";

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
  const downloadQuality = useSettingsStore((s) => s.downloadQuality);
  const setDownloadQuality = useSettingsStore((s) => s.setDownloadQuality);

  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);

  useEffect(() => {
    refresh();
  }, [refresh]);

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
          <select
            className="sort-select downloads-quality-select"
            value={downloadQuality}
            onChange={(e) =>
              setDownloadQuality(e.target.value as DownloadQuality).catch((err) =>
                setError(String(err)),
              )
            }
            aria-label="Download quality"
            title="Lossless downloads the original file as-is. The other options transcode lossless tracks to Ogg/Opus at the chosen bitrate; already-lossy tracks always download as-is."
          >
            <option value="original">Lossless</option>
            <option value="kbps320">320 kbps</option>
            <option value="kbps256">256 kbps</option>
            <option value="kbps192">192 kbps</option>
            <option value="kbps128">128 kbps</option>
          </select>
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

          <PlaylistDownloadSection onError={setError} />

          <BookmarkDownloadSection onError={setError} />

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

interface InProgressProps {
  inProgress: InProgressDownload | null;
  liveProgress: { bytesWritten: number; totalBytes: number | null } | null;
  queueLen: number;
  onCancelAll: () => Promise<void>;
}

function InProgressSection(props: InProgressProps) {
  // Early-return must happen before any hooks to satisfy rules-of-hooks.
  // Wrapping the hook-bearing body in a separate component keeps the hook
  // call count stable once the section mounts.
  if (!props.inProgress && props.queueLen === 0) return null;
  return <InProgressSectionBody {...props} />;
}

function InProgressSectionBody({
  inProgress,
  liveProgress,
  queueLen,
  onCancelAll,
}: InProgressProps) {
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

/// Playlists (regular and smart) as expandable download rows. The size
/// estimate is fetched on expand, not eagerly — each estimate is a
/// per-playlist items fetch, so warming every row on open would hammer
/// the server for numbers nobody asked about.
function PlaylistDownloadSection({ onError }: { onError: (msg: string | null) => void }) {
  const startPlaylistDownload = useDownloadsStore((s) => s.startPlaylistDownload);
  const estimatePlaylist = useDownloadsStore((s) => s.estimatePlaylist);
  // The backend sizes an estimate against the current download quality, so
  // changing it while a row is open invalidates whatever is on screen.
  const downloadQuality = useSettingsStore((s) => s.downloadQuality);
  const [playlists, setPlaylists] = useState<Playlist[] | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [estimate, setEstimate] = useState<PlaylistDownloadEstimate | null>(null);
  const [estimating, setEstimating] = useState(false);

  useEffect(() => {
    getPlaylists()
      .then(setPlaylists)
      .catch(() => setPlaylists([]));
  }, []);

  useEffect(() => {
    if (!expandedId) {
      setEstimate(null);
      return;
    }
    let cancelled = false;
    setEstimating(true);
    setEstimate(null);
    estimatePlaylist(expandedId)
      .then((e) => {
        if (!cancelled) setEstimate(e);
      })
      .catch(() => {
        if (!cancelled) setEstimate(null);
      })
      .finally(() => {
        if (!cancelled) setEstimating(false);
      });
    return () => {
      cancelled = true;
    };
  }, [expandedId, estimatePlaylist, downloadQuality]);

  const handleDownload = async (sourceId: string) => {
    try {
      const n = await startPlaylistDownload(sourceId);
      if (n === 0) onError("No downloadable tracks in that playlist.");
      setExpandedId(null);
    } catch (e) {
      onError(String(e));
    }
  };

  return (
    <section className="downloads-section">
      <h3 className="downloads-section-title">Playlists</h3>
      {playlists === null ? (
        <div className="downloads-empty">Loading…</div>
      ) : playlists.length === 0 ? (
        <div className="downloads-empty">No playlists yet.</div>
      ) : (
        <ul className="downloads-list">
          {playlists.map((p) => {
            const expanded = expandedId === p.sourceId;
            return (
              <li key={p.sourceId} className="downloads-source-item">
                <button
                  className="downloads-source-row"
                  aria-expanded={expanded}
                  onClick={() => setExpandedId(expanded ? null : p.sourceId)}
                >
                  <span className="downloads-source-name">{p.title}</span>
                  {p.smart && <span className="mobile-lists-smart-badge">SMART</span>}
                  {p.trackCount != null && (
                    <span className="downloads-source-count">{p.trackCount}</span>
                  )}
                </button>
                {expanded && (
                  <div className="downloads-source-detail">
                    <button className="settings-btn" onClick={() => handleDownload(p.sourceId)}>
                      Download
                      {estimate && (
                        <span className="downloads-btn-detail">
                          {" "}
                          ({estimate.trackCount} track{estimate.trackCount === 1 ? "" : "s"}, ~
                          {formatBytes(estimate.totalBytes)})
                        </span>
                      )}
                      {estimating && <span className="downloads-btn-detail"> (estimating…)</span>}
                    </button>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

/// Smart Filters as expandable download rows — same interaction shape as
/// the playlist rows above so the two source lists read as one surface.
function BookmarkDownloadSection({ onError }: { onError: (msg: string | null) => void }) {
  const bookmarks = useSettingsStore((s) => s.bookmarks);
  const startBookmarkDownload = useDownloadsStore((s) => s.startBookmarkDownload);
  const estimateBookmark = useDownloadsStore((s) => s.estimateBookmark);
  const downloadQuality = useSettingsStore((s) => s.downloadQuality);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [estimate, setEstimate] = useState<BookmarkDownloadEstimate | null>(null);
  const [estimating, setEstimating] = useState(false);

  const expanded = bookmarks.find((b) => b.id === expandedId);

  useEffect(() => {
    if (!expanded) {
      setEstimate(null);
      return;
    }
    let cancelled = false;
    setEstimating(true);
    setEstimate(null);
    estimateBookmark(expanded.filters)
      .then((e) => {
        if (!cancelled) setEstimate(e);
      })
      .catch(() => {
        if (!cancelled) setEstimate(null);
      })
      .finally(() => {
        if (!cancelled) setEstimating(false);
      });
    return () => {
      cancelled = true;
    };
    // Key on the bookmark id — the underlying filter shape is stable per
    // bookmark, so we don't need a deep-compare here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [expanded?.id, estimateBookmark, downloadQuality]);

  const handleDownload = useCallback(async () => {
    if (!expanded) return;
    try {
      const n = await startBookmarkDownload(expanded.filters);
      if (n === 0) onError("No downloadable tracks for that Smart Filter.");
      setExpandedId(null);
    } catch (e) {
      onError(String(e));
    }
  }, [expanded, startBookmarkDownload, onError]);

  return (
    <section className="downloads-section">
      <h3 className="downloads-section-title">Smart Filters</h3>
      {bookmarks.length === 0 ? (
        <div className="downloads-empty">
          No Smart Filters yet — set a filter and tap Save in the filter panel.
        </div>
      ) : (
        <ul className="downloads-list">
          {bookmarks.map((b) => {
            const isOpen = expandedId === b.id;
            return (
              <li key={b.id} className="downloads-source-item">
                <button
                  className="downloads-source-row"
                  aria-expanded={isOpen}
                  onClick={() => setExpandedId(isOpen ? null : b.id)}
                >
                  <span className="downloads-source-name">{b.name}</span>
                </button>
                {isOpen && (
                  <div className="downloads-source-detail">
                    <div className="bookmark-download-summary">
                      {describeFilters(filtersFromBookmark(b))}
                    </div>
                    <button className="settings-btn" onClick={handleDownload}>
                      Download
                      {estimate && (
                        <span className="downloads-btn-detail">
                          {" "}
                          ({estimate.trackCount} track{estimate.trackCount === 1 ? "" : "s"}, ~
                          {formatBytes(estimate.totalBytes)})
                        </span>
                      )}
                      {estimating && <span className="downloads-btn-detail"> (estimating…)</span>}
                    </button>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </section>
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
