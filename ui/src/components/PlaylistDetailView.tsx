import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useLibraryStore } from "../stores/libraryStore";
import { useDownloadsStore } from "../stores/downloadsStore";
import { useToastStore } from "./Toast";
import type { PlaylistDownloadEstimate, PlaylistItem, Track } from "../lib/types";
import {
  ART_SIZE,
  appendToQueue,
  deletePlaylist,
  getAlbum,
  getPlaylistItems,
  movePlaylistItem,
  playTracks,
  removePlaylistItem,
  renamePlaylist,
} from "../lib/commands";
import { useListReorder } from "../lib/useListReorder";
import { useArtUrl } from "../lib/useArtUrl";
import { shuffleTracks } from "../lib/shuffle";
import { formatBytes, formatDuration, formatLongDuration } from "../lib/format";
import { IconClose, IconMoreDots, IconMusicNote, IconPlay, IconShuffle } from "./Icons";

const ROW_HEIGHT = 44;

/** Per-row album thumbnail (own component so each row gets its own art
 * hook instance). */
function RowArt({ thumb }: { thumb: string | null }) {
  const { artSrc, artErr, setArtErr } = useArtUrl(thumb, ART_SIZE.SMALL);
  return (
    <div className="playlist-row-art">
      {artSrc && !artErr ? (
        <img src={artSrc} alt="" onError={() => setArtErr(true)} />
      ) : (
        <div className="playlist-row-art-ph">
          <IconMusicNote size={12} />
        </div>
      )}
    </div>
  );
}

/**
 * Desktop playlist detail (main content area, driven by
 * `libraryStore.browsePlaylist`). Row click plays the playlist from that
 * entry; the `::` handle drags to reorder (regular playlists only). The
 * header `…` menu carries Download / Rename / Delete; each row's `…` menu
 * carries navigation and the track favourite.
 */
export default function PlaylistDetailView() {
  const playlist = useLibraryStore((s) => s.browsePlaylist);
  const [items, setItems] = useState<PlaylistItem[] | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [confirmDownload, setConfirmDownload] = useState(false);
  const [dlEstimate, setDlEstimate] = useState<PlaylistDownloadEstimate | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const [renameBusy, setRenameBusy] = useState(false);
  const [rowMenu, setRowMenu] = useState<number | null>(null);
  const confirmTimer = useRef<number | null>(null);
  const { artSrc, artErr, setArtErr } = useArtUrl(playlist?.thumb, ART_SIZE.MEDIUM);

  useEffect(() => {
    setItems(null);
    setMenuOpen(false);
    setConfirmDownload(false);
    setConfirmDelete(false);
    setRenaming(false);
    setRowMenu(null);
    if (!playlist) return;
    let cancelled = false;
    getPlaylistItems(playlist.sourceId)
      .then((list) => {
        if (!cancelled) setItems(list);
      })
      .catch(() => {
        if (!cancelled) setItems([]);
      });
    return () => {
      cancelled = true;
    };
  }, [playlist?.sourceId]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(
    () => () => {
      if (confirmTimer.current !== null) window.clearTimeout(confirmTimer.current);
    },
    [],
  );

  // Close any open dropdown on outside click (same pattern as
  // AlbumDetailView's menus).
  useEffect(() => {
    if (!menuOpen && !confirmDownload && rowMenu === null) return;
    const handler = (e: MouseEvent) => {
      if (!(e.target as Element).closest(".pl-menu-wrap")) {
        setMenuOpen(false);
        setConfirmDownload(false);
        setConfirmDelete(false);
        setRowMenu(null);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [menuOpen, confirmDownload, rowMenu]);

  // Size estimate for the download confirm, fetched when it opens.
  useEffect(() => {
    if (!confirmDownload || !playlist) {
      setDlEstimate(null);
      return;
    }
    let cancelled = false;
    useDownloadsStore
      .getState()
      .estimatePlaylist(playlist.sourceId)
      .then((e) => {
        if (!cancelled) setDlEstimate(e);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [confirmDownload, playlist]);

  const reorder = (from: number, to: number) => {
    if (!playlist || !items) return;
    // Smart-playlist entries carry no per-item id (the handles aren't
    // rendered for them, so this is a type-level backstop).
    const movedId = items[from]?.playlistItemId;
    if (movedId == null) return;
    const next = [...items];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    setItems(next);
    const afterItemId = to > 0 ? next[to - 1].playlistItemId : null;
    movePlaylistItem(playlist.sourceId, movedId, afterItemId)
      .then(setItems)
      .catch(() => {
        useToastStore.getState().show("Couldn't reorder playlist");
        getPlaylistItems(playlist.sourceId)
          .then(setItems)
          .catch(() => {});
      });
  };

  const { setRowRef, handleProps } = useListReorder({
    count: items?.length ?? 0,
    rowHeight: ROW_HEIGHT,
    onReorder: reorder,
  });

  if (!playlist) return null;

  const play = (startAt: number) => {
    if (!items || items.length === 0) return;
    playTracks(
      items.map((i) => i.track),
      startAt,
    ).catch(() => {});
  };

  const shuffle = () => {
    if (!items || items.length === 0) return;
    playTracks(shuffleTracks(items.map((i) => i.track)), 0).catch(() => {});
  };

  const removeRow = (index: number) => {
    if (!items) return;
    const item = items[index];
    if (item.playlistItemId == null) return;
    setItems(items.filter((_, i) => i !== index));
    removePlaylistItem(playlist.sourceId, item.playlistItemId)
      .then(setItems)
      .catch(() => {
        useToastStore.getState().show("Couldn't remove track");
        getPlaylistItems(playlist.sourceId)
          .then(setItems)
          .catch(() => {});
      });
  };

  const handleDownload = () => {
    setMenuOpen(false);
    setConfirmDownload(false);
    useDownloadsStore
      .getState()
      .startPlaylistDownload(playlist.sourceId)
      .then((n) => {
        useToastStore
          .getState()
          .show(n === 0 ? "No downloadable tracks" : `Queued ${n} track${n === 1 ? "" : "s"}`);
      })
      .catch(() => {
        useToastStore.getState().show("Couldn't start download");
      });
  };

  const handleDelete = () => {
    if (!confirmDelete) {
      setConfirmDelete(true);
      confirmTimer.current = window.setTimeout(() => setConfirmDelete(false), 3000);
      return;
    }
    setMenuOpen(false);
    deletePlaylist(playlist.sourceId)
      .then(() => {
        useToastStore.getState().show(`Deleted “${playlist.title}”`);
        useLibraryStore.setState({ browsePlaylist: null });
      })
      .catch(() => useToastStore.getState().show("Couldn't delete playlist"));
  };

  const handleRename = () => {
    const trimmed = renameValue.trim();
    if (!trimmed || renameBusy) return;
    if (trimmed === playlist.title) {
      setRenaming(false);
      return;
    }
    setRenameBusy(true);
    renamePlaylist(playlist.sourceId, trimmed)
      .then((updated) => {
        // The detail view renders from browsePlaylist; the sidebar refetches
        // lazily, so patching the store is all the UI needs.
        useLibraryStore.setState({ browsePlaylist: updated });
        setRenaming(false);
        setRenameBusy(false);
      })
      .catch(() => {
        useToastStore.getState().show("Couldn't rename playlist");
        setRenameBusy(false);
      });
  };

  const goToAlbum = (track: Track) => {
    if (!track.albumKey) return;
    getAlbum(track.albumKey)
      .then((album) => {
        if (album) void useLibraryStore.getState().openAlbumDetail(album);
        else useToastStore.getState().show("Album isn't in the library");
      })
      .catch(() => {});
  };

  const toggleFav = (index: number) => {
    if (!items) return;
    const track = items[index].track;
    // libraryStore owns the IPC + cross-store patches; mirror the flip into
    // this view's local copy so the menu label stays honest.
    void useLibraryStore.getState().toggleTrackFav(track);
    setItems(
      items.map((it, i) =>
        i === index ? { ...it, track: { ...it.track, isFavourite: !track.isFavourite } } : it,
      ),
    );
  };

  const count = items?.length ?? playlist.trackCount;
  const subtitleBits: string[] = [];
  if (count != null) subtitleBits.push(`${count} track${count === 1 ? "" : "s"}`);
  if (playlist.duration) subtitleBits.push(formatLongDuration(playlist.duration));
  if (playlist.smart) subtitleBits.push("Smart Playlist");

  return (
    <div className="playlist-view">
      <div className="playlist-view-header">
        <div className="playlist-view-art">
          {artSrc && !artErr ? (
            <img src={artSrc} alt={playlist.title} onError={() => setArtErr(true)} />
          ) : (
            <div className="playlist-view-art-ph">
              <IconMusicNote size={24} />
            </div>
          )}
        </div>
        <div className="playlist-view-titles">
          <h2 className="playlist-view-title">{playlist.title}</h2>
          {subtitleBits.length > 0 && (
            <div className="playlist-view-subtitle">{subtitleBits.join(" · ")}</div>
          )}
        </div>
        <div className="playlist-view-actions">
          <button className="playlist-view-btn" onClick={() => play(0)}>
            <IconPlay size={12} /> Play
          </button>
          <button className="playlist-view-btn" onClick={shuffle}>
            <IconShuffle size={12} /> Shuffle
          </button>
          <div className="pl-menu-wrap">
            <button
              className={`playlist-view-btn icon${menuOpen || confirmDownload ? " active" : ""}`}
              title="More actions"
              onClick={() => {
                setConfirmDownload(false);
                setConfirmDelete(false);
                setMenuOpen((v) => !v);
              }}
            >
              <IconMoreDots size={14} />
            </button>
            {menuOpen && (
              <div className="adv-dropdown">
                <button
                  onClick={() => {
                    setMenuOpen(false);
                    setConfirmDownload(true);
                  }}
                >
                  Download Playlist
                </button>
                <button
                  onClick={() => {
                    setMenuOpen(false);
                    setRenameValue(playlist.title);
                    setRenaming(true);
                  }}
                >
                  Rename Playlist
                </button>
                <button className="destructive" onClick={handleDelete}>
                  {confirmDelete ? "Confirm delete?" : "Delete Playlist"}
                </button>
              </div>
            )}
            {confirmDownload && (
              <div className="adv-dropdown playlist-dl-confirm">
                <div className="playlist-dl-confirm-text">
                  {dlEstimate
                    ? `Download ${dlEstimate.trackCount} track${
                        dlEstimate.trackCount === 1 ? "" : "s"
                      } (~${formatBytes(dlEstimate.totalBytes)})?`
                    : "Download this playlist?"}
                </div>
                <button onClick={handleDownload}>Download</button>
                <button onClick={() => setConfirmDownload(false)}>Cancel</button>
              </div>
            )}
          </div>
        </div>
      </div>

      {items === null ? (
        <div className="empty-state">Loading…</div>
      ) : items.length === 0 ? (
        <div className="empty-state">This playlist is empty</div>
      ) : (
        <div className="playlist-scroll">
          {items.map((item, index) => {
            const isNearBottom = index >= items.length - 3 && items.length > 4;
            return (
              <div
                key={item.playlistItemId ?? index}
                className="playlist-row desktop"
                ref={(el) => setRowRef(index, el)}
              >
                <button className="playlist-row-body" onClick={() => play(index)}>
                  <span className="playlist-row-num">{index + 1}</span>
                  <RowArt thumb={item.track.thumb} />
                  <div className="playlist-row-info">
                    <div className="playlist-row-title">{item.track.title}</div>
                    <div className="playlist-row-artist">
                      {item.track.trackArtist || item.track.artistName} — {item.track.albumTitle}
                    </div>
                  </div>
                  <span className="playlist-row-duration">
                    {formatDuration(item.track.duration)}
                  </span>
                </button>
                <button
                  className="playlist-row-queue"
                  title="Add to queue"
                  onClick={() => appendToQueue([item.track]).catch(() => {})}
                >
                  +
                </button>
                <div className="pl-menu-wrap">
                  <button
                    className="playlist-row-dots"
                    title="Track actions"
                    onClick={() => setRowMenu((prev) => (prev === index ? null : index))}
                  >
                    <IconMoreDots size={14} />
                  </button>
                  {rowMenu === index && (
                    <div className={`adv-dropdown${isNearBottom ? " up" : ""}`}>
                      <button
                        onClick={() => {
                          setRowMenu(null);
                          goToAlbum(item.track);
                        }}
                      >
                        Go to Album
                      </button>
                      <button
                        onClick={() => {
                          setRowMenu(null);
                          useLibraryStore.getState().loadAlbumsForArtistName(item.track.artistName);
                        }}
                      >
                        Go to Artist
                      </button>
                      <button
                        onClick={() => {
                          setRowMenu(null);
                          toggleFav(index);
                        }}
                      >
                        {item.track.isFavourite ? "Unfavourite Track" : "Favourite Track"}
                      </button>
                    </div>
                  )}
                </div>
                {!playlist.smart && (
                  <>
                    <button
                      className="playlist-row-remove"
                      title="Remove from playlist"
                      onClick={() => removeRow(index)}
                    >
                      <IconClose size={12} />
                    </button>
                    <span className="playlist-grab" aria-label="Reorder" {...handleProps(index)}>
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
                        <circle cx="9" cy="6" r="1.8" />
                        <circle cx="15" cy="6" r="1.8" />
                        <circle cx="9" cy="12" r="1.8" />
                        <circle cx="15" cy="12" r="1.8" />
                        <circle cx="9" cy="18" r="1.8" />
                        <circle cx="15" cy="18" r="1.8" />
                      </svg>
                    </span>
                  </>
                )}
              </div>
            );
          })}
        </div>
      )}

      {renaming &&
        createPortal(
          <div
            className="settings-backdrop"
            onClick={(e) => {
              if (e.target === e.currentTarget) setRenaming(false);
            }}
          >
            <div
              className="settings-panel glass picker-modal"
              role="dialog"
              aria-modal="true"
              aria-labelledby="playlist-rename-title"
            >
              <div className="settings-header">
                <h2 id="playlist-rename-title">Rename Playlist</h2>
                <button
                  className="settings-close"
                  onClick={() => setRenaming(false)}
                  aria-label="Close"
                >
                  x
                </button>
              </div>
              <form
                className="settings-body"
                onSubmit={(e) => {
                  e.preventDefault();
                  handleRename();
                }}
              >
                <input
                  className="bookmark-save-input"
                  type="text"
                  value={renameValue}
                  autoFocus
                  placeholder="Playlist name"
                  onChange={(e) => setRenameValue(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape") {
                      e.preventDefault();
                      setRenaming(false);
                    }
                  }}
                />
                <div className="bookmark-actions">
                  <div style={{ flex: 1 }} />
                  <button type="button" className="bookmark-btn" onClick={() => setRenaming(false)}>
                    Cancel
                  </button>
                  <button
                    type="submit"
                    className="bookmark-btn bookmark-save"
                    disabled={!renameValue.trim() || renameBusy}
                  >
                    {renameBusy ? "Renaming…" : "Rename"}
                  </button>
                </div>
              </form>
            </div>
          </div>,
          document.body,
        )}
    </div>
  );
}
