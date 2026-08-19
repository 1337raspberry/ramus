import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type {
  CrateRecipe,
  Playlist,
  PlaylistDownloadEstimate,
  PlaylistItem,
  Track,
} from "../lib/types";
import {
  ART_SIZE,
  deletePlaylist,
  getAlbum,
  getCrateRecipe,
  getPlaylistItems,
  movePlaylistItem,
  playTracks,
  regenerateCratePlaylist,
  removePlaylistItem,
  renamePlaylist,
  appendToQueue,
} from "../lib/commands";
import { refreshQueue } from "../lib/refreshQueue";
import CrateBuilder from "./CrateBuilder";
import { useListReorder } from "../lib/useListReorder";
import { useLongPress } from "../lib/useLongPress";
import { useSwipeToDelete } from "../lib/useSwipeToDelete";
import { useArtUrl } from "../lib/useArtUrl";
import { shuffleTracks } from "../lib/shuffle";
import { useLibraryStore } from "../stores/libraryStore";
import { useDownloadsStore } from "../stores/downloadsStore";
import { useToastStore } from "../components/Toast";
import { pushBackHandler } from "../lib/backHandler";
import { formatBytes, formatDuration, formatLongDuration } from "../lib/format";
import {
  IconChevronLeft,
  IconMoreDots,
  IconMusicNote,
  IconPlay,
  IconShuffle,
} from "../components/Icons";

const ROW_HEIGHT = 56;

interface Props {
  playlist: Playlist;
  onBack: () => void;
  /** Navigate to an artist's album grid. Owned by MobileApp so it can
   * breadcrumb the way back here — plain back from the grid returns to
   * this playlist, matching the album-detail overlay's behaviour. */
  onGoToArtist: (artistName: string) => void;
}

/** One row's body: album thumb + titles + duration. Tap plays the playlist
 * from here, long-press opens the per-row sheet (same sheet as the `…`
 * button beside it). */
function RowBody({
  item,
  onPlay,
  onSheet,
}: {
  item: PlaylistItem;
  onPlay: () => void;
  onSheet: () => void;
}) {
  const longPress = useLongPress({ onLongPress: onSheet, onClick: onPlay });
  const { artSrc, artErr, setArtErr } = useArtUrl(item.track.thumb, ART_SIZE.SMALL);
  return (
    <button className="playlist-row-body" {...longPress}>
      <div className="playlist-row-art">
        {artSrc && !artErr ? (
          <img src={artSrc} alt="" onError={() => setArtErr(true)} />
        ) : (
          <div className="playlist-row-art-ph">
            <IconMusicNote size={16} />
          </div>
        )}
      </div>
      <div className="playlist-row-info">
        <div className="playlist-row-title">{item.track.title}</div>
        <div className="playlist-row-artist">{item.track.trackArtist || item.track.artistName}</div>
      </div>
      <span className="playlist-row-duration">{formatDuration(item.track.duration)}</span>
    </button>
  );
}

/**
 * Playlist detail: hero (composite art, meta, play/shuffle) over the ordered
 * track list. Drag-to-reorder via the `::` grab handles (regular playlists
 * only — smart playlists are display/play only). Reorder is optimistic; the
 * server's refreshed list reconciles on landing.
 */
export default function MobilePlaylistDetail({ playlist, onBack, onGoToArtist }: Props) {
  const [items, setItems] = useState<PlaylistItem[] | null>(null);
  const [showMenu, setShowMenu] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const [renameBusy, setRenameBusy] = useState(false);
  const [rowSheet, setRowSheet] = useState<number | null>(null);
  const [confirmDownload, setConfirmDownload] = useState(false);
  // Non-null only for a generated playlist; drives the Regenerate and Edit
  // actions (crates take Edit in place of Rename — the title is derived from
  // the recipe, so it's changed by changing the rules).
  const [crateRecipe, setCrateRecipe] = useState<CrateRecipe | null>(null);
  const [regenerating, setRegenerating] = useState(false);
  const [editingCrate, setEditingCrate] = useState(false);
  const [dlEstimate, setDlEstimate] = useState<PlaylistDownloadEstimate | null>(null);
  const { artSrc, artErr, setArtErr } = useArtUrl(playlist.thumb, ART_SIZE.MEDIUM);

  useEffect(() => {
    let cancelled = false;
    // Smart playlists are server-computed and can't be a crate, so don't spend
    // a lookup on them.
    if (playlist.smart) {
      setCrateRecipe(null);
    } else {
      getCrateRecipe(playlist.sourceId)
        .then((r) => {
          if (!cancelled) setCrateRecipe(r);
        })
        .catch(() => {
          if (!cancelled) setCrateRecipe(null);
        });
    }
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
  }, [playlist.sourceId]);

  useEffect(() => {
    if (!showMenu && !confirmDelete && !renaming && !confirmDownload && rowSheet === null) return;
    return pushBackHandler(() => {
      setShowMenu(false);
      setConfirmDelete(false);
      setRenaming(false);
      setConfirmDownload(false);
      setRowSheet(null);
      return true;
    });
  }, [showMenu, confirmDelete, renaming, confirmDownload, rowSheet]);

  // Size estimate for the download confirm sheet, fetched when it opens.
  useEffect(() => {
    if (!confirmDownload) {
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
  }, [confirmDownload, playlist.sourceId]);

  const handleDownload = () => {
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

  const reorder = (from: number, to: number) => {
    if (!items) return;
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

  // Swipe-left-to-remove on regular playlists (smart entries can't be
  // removed, so their rows don't take the gesture at all).
  const swipe = useSwipeToDelete({ onDelete: (i) => removeRow(i) });

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
    // Optimistic removal; the refreshed server list reconciles.
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

  const handleRegenerate = () => {
    if (regenerating) return;
    setRegenerating(true);
    regenerateCratePlaylist(playlist.sourceId)
      .then((next) => {
        setItems(next);
        setShowMenu(false);
        useToastStore.getState().show(`Regenerated — ${next.length} tracks`);
      })
      .catch((e) => {
        useToastStore.getState().show(String(e) || "Couldn't regenerate");
      })
      .finally(() => setRegenerating(false));
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
        // The detail view renders from browsePlaylist; the hub refetches
        // lazily on view. The revision bump is for any list mounted
        // alongside this one, which is the desktop sidebar's situation.
        useLibraryStore.setState({ browsePlaylist: updated });
        useLibraryStore.getState().bumpPlaylistsRevision();
        setRenaming(false);
        setRenameBusy(false);
      })
      .catch(() => {
        useToastStore.getState().show("Couldn't rename playlist");
        setRenameBusy(false);
      });
  };

  const handleDelete = () => {
    deletePlaylist(playlist.sourceId)
      .then(() => {
        useToastStore.getState().show(`Deleted “${playlist.title}”`);
        useLibraryStore.getState().bumpPlaylistsRevision();
        onBack();
      })
      .catch(() => useToastStore.getState().show("Couldn't delete playlist"));
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

  const goToArtist = (track: Track) => {
    onGoToArtist(track.artistName);
  };

  const toggleFav = (index: number) => {
    if (!items) return;
    const track = items[index].track;
    // libraryStore owns the IPC + cross-store patches; mirror the flip into
    // this view's local copy so the sheet label stays honest.
    void useLibraryStore.getState().toggleTrackFav(track);
    setItems(
      items.map((it, i) =>
        i === index ? { ...it, track: { ...it.track, isFavourite: !track.isFavourite } } : it,
      ),
    );
  };

  const subtitleBits: string[] = [];
  // Both figures come from the loaded entries rather than the passed-in
  // playlist, which is a snapshot from the list view: regenerating a crate or
  // removing a track changes the contents without it, leaving the old
  // runtime on screen next to a freshly correct count.
  const count = items?.length ?? playlist.trackCount;
  const duration = items
    ? items.reduce((total, item) => total + item.track.duration, 0)
    : playlist.duration;
  if (count != null) subtitleBits.push(`${count} track${count === 1 ? "" : "s"}`);
  if (duration) subtitleBits.push(formatLongDuration(duration));
  if (playlist.smart) subtitleBits.push("Smart Playlist");

  return (
    <div className="mobile-screen">
      <header className="mobile-header mobile-header-4col">
        <button className="mobile-header-circle" onClick={onBack} aria-label="Back">
          <IconChevronLeft size={22} />
        </button>
        <div className="mobile-header-title-wrap">
          <div className="mobile-header-title">
            <span>{playlist.title}</span>
          </div>
        </div>
        <span />
        <button
          className="mobile-header-circle"
          onClick={() => setShowMenu(true)}
          aria-label="Playlist actions"
        >
          <IconMoreDots size={22} />
        </button>
      </header>

      <div className="mobile-detail-hero playlist-hero">
        <div className="mobile-detail-art">
          {artSrc && !artErr ? (
            <img src={artSrc} alt={playlist.title} onError={() => setArtErr(true)} />
          ) : (
            <div className="mobile-detail-art-ph">
              <IconMusicNote size={32} />
            </div>
          )}
        </div>
        <div className="mobile-detail-meta">
          {subtitleBits.map((bit) => (
            <div key={bit} className="playlist-hero-line">
              {bit}
            </div>
          ))}
          <div className="mobile-detail-actions">
            <button
              className="mobile-detail-play"
              aria-label="Play playlist"
              onClick={() => play(0)}
            >
              <IconPlay size={22} />
            </button>
            <button
              className="mobile-detail-play playlist-shuffle"
              aria-label="Shuffle playlist"
              onClick={shuffle}
            >
              <IconShuffle size={20} />
            </button>
          </div>
        </div>
      </div>

      {items === null ? (
        <div className="mobile-empty">Loading…</div>
      ) : items.length === 0 ? (
        <div className="mobile-empty">This playlist is empty</div>
      ) : (
        <div className="playlist-scroll">
          {items.map((item, index) => (
            <div
              key={item.playlistItemId ?? index}
              className={`playlist-row${playlist.smart ? "" : " swipe-row"}`}
              ref={(el) => setRowRef(index, el)}
            >
              <div
                className="playlist-row-content swipe-row-content"
                ref={(el) => swipe.setContentRef(index, el)}
                {...(playlist.smart ? {} : swipe.contentProps(index))}
              >
                <RowBody
                  item={item}
                  onPlay={() => play(index)}
                  onSheet={() => setRowSheet(index)}
                />
                <button
                  className="playlist-row-menu"
                  aria-label="Track actions"
                  onClick={() => setRowSheet(index)}
                >
                  <IconMoreDots size={18} />
                </button>
                {!playlist.smart && (
                  <span className="playlist-grab" aria-label="Reorder" {...handleProps(index)}>
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
                      <circle cx="9" cy="6" r="1.6" />
                      <circle cx="15" cy="6" r="1.6" />
                      <circle cx="9" cy="12" r="1.6" />
                      <circle cx="15" cy="12" r="1.6" />
                      <circle cx="9" cy="18" r="1.6" />
                      <circle cx="15" cy="18" r="1.6" />
                    </svg>
                  </span>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {editingCrate && crateRecipe && (
        <CrateBuilder
          existing={{ sourceId: playlist.sourceId, recipe: crateRecipe }}
          onDismiss={() => setEditingCrate(false)}
          onSaved={(update, recipe) => {
            setEditingCrate(false);
            setItems(update.items);
            setCrateRecipe(recipe);
            // Same write-back as a rename: the detail view renders from
            // browsePlaylist, and the revision bump reaches any list mounted
            // alongside (the desktop sidebar's situation).
            useLibraryStore.setState({ browsePlaylist: update.playlist });
            useLibraryStore.getState().bumpPlaylistsRevision();
          }}
        />
      )}

      {showMenu &&
        createPortal(
          <div
            className="mobile-action-sheet-backdrop"
            onClick={(e) => {
              if (e.target === e.currentTarget) setShowMenu(false);
            }}
          >
            <div className="mobile-action-sheet">
              <div className="mobile-action-sheet-group">
                <button
                  onClick={() => {
                    setShowMenu(false);
                    setConfirmDownload(true);
                  }}
                >
                  Download Playlist
                </button>
                {crateRecipe && (
                  <button disabled={regenerating} onClick={handleRegenerate}>
                    {regenerating ? "Regenerating…" : "Regenerate Crate"}
                  </button>
                )}
                {crateRecipe ? (
                  <button
                    onClick={() => {
                      setShowMenu(false);
                      setEditingCrate(true);
                    }}
                  >
                    Edit Crate
                  </button>
                ) : (
                  <button
                    onClick={() => {
                      setShowMenu(false);
                      setRenameValue(playlist.title);
                      setRenaming(true);
                    }}
                  >
                    Rename Playlist
                  </button>
                )}
                <button
                  className="destructive"
                  onClick={() => {
                    setShowMenu(false);
                    setConfirmDelete(true);
                  }}
                >
                  Delete Playlist
                </button>
              </div>
              <button className="mobile-action-sheet-cancel" onClick={() => setShowMenu(false)}>
                Cancel
              </button>
            </div>
          </div>,
          document.body,
        )}

      {confirmDownload &&
        createPortal(
          <div
            className="mobile-action-sheet-backdrop"
            onClick={(e) => {
              if (e.target === e.currentTarget) setConfirmDownload(false);
            }}
          >
            <div className="mobile-action-sheet">
              <div className="mobile-action-sheet-group">
                <div className="mobile-action-sheet-header">
                  {dlEstimate
                    ? `Download ${dlEstimate.trackCount} track${
                        dlEstimate.trackCount === 1 ? "" : "s"
                      } (~${formatBytes(dlEstimate.totalBytes)})?`
                    : "Download this playlist?"}
                </div>
                <button onClick={handleDownload}>Download</button>
              </div>
              <button
                className="mobile-action-sheet-cancel"
                onClick={() => setConfirmDownload(false)}
              >
                Cancel
              </button>
            </div>
          </div>,
          document.body,
        )}

      {renaming &&
        createPortal(
          <div
            className="mobile-action-sheet-backdrop text-entry"
            onClick={(e) => {
              if (e.target === e.currentTarget) setRenaming(false);
            }}
          >
            <div className="mobile-action-sheet">
              <div className="mobile-action-sheet-group">
                <div className="mobile-action-sheet-header">Rename “{playlist.title}”</div>
                <div className="playlist-name-entry">
                  <input
                    className="playlist-name-input"
                    type="text"
                    value={renameValue}
                    autoFocus
                    placeholder="Playlist name"
                    onChange={(e) => setRenameValue(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") handleRename();
                    }}
                  />
                  <button
                    className="playlist-name-create"
                    disabled={!renameValue.trim() || renameBusy}
                    onClick={handleRename}
                  >
                    Rename
                  </button>
                </div>
              </div>
              <button className="mobile-action-sheet-cancel" onClick={() => setRenaming(false)}>
                Cancel
              </button>
            </div>
          </div>,
          document.body,
        )}

      {confirmDelete &&
        createPortal(
          <div
            className="mobile-action-sheet-backdrop"
            onClick={(e) => {
              if (e.target === e.currentTarget) setConfirmDelete(false);
            }}
          >
            <div className="mobile-action-sheet">
              <div className="mobile-action-sheet-group">
                <div className="mobile-action-sheet-header">
                  Delete “{playlist.title}” from Plex? This affects every app signed into this
                  account.
                </div>
                <button
                  className="destructive"
                  onClick={() => {
                    setConfirmDelete(false);
                    handleDelete();
                  }}
                >
                  Delete
                </button>
              </div>
              <button
                className="mobile-action-sheet-cancel"
                onClick={() => setConfirmDelete(false)}
              >
                Cancel
              </button>
            </div>
          </div>,
          document.body,
        )}

      {rowSheet !== null &&
        items &&
        items[rowSheet] &&
        createPortal(
          <div
            className="mobile-action-sheet-backdrop"
            onClick={(e) => {
              if (e.target === e.currentTarget) setRowSheet(null);
            }}
          >
            <div className="mobile-action-sheet">
              <div className="mobile-action-sheet-group">
                <div className="mobile-action-sheet-header">{items[rowSheet].track.title}</div>
                <button
                  onClick={() => {
                    const idx = rowSheet;
                    setRowSheet(null);
                    play(idx);
                  }}
                >
                  Play from Here
                </button>
                <button
                  onClick={() => {
                    const track = items[rowSheet].track;
                    setRowSheet(null);
                    void appendToQueue([track])
                      .then(refreshQueue)
                      .catch(() => {});
                  }}
                >
                  Add to Queue
                </button>
                <button
                  onClick={() => {
                    const track = items[rowSheet].track;
                    setRowSheet(null);
                    goToAlbum(track);
                  }}
                >
                  Go to Album
                </button>
                <button
                  onClick={() => {
                    const track = items[rowSheet].track;
                    setRowSheet(null);
                    goToArtist(track);
                  }}
                >
                  Go to Artist
                </button>
                <button
                  onClick={() => {
                    const idx = rowSheet;
                    setRowSheet(null);
                    toggleFav(idx);
                  }}
                >
                  {items[rowSheet].track.isFavourite ? "Unfavourite Track" : "Favourite Track"}
                </button>
                {!playlist.smart && (
                  <button
                    className="destructive"
                    onClick={() => {
                      const idx = rowSheet;
                      setRowSheet(null);
                      removeRow(idx);
                    }}
                  >
                    Remove from Playlist
                  </button>
                )}
              </div>
              <button className="mobile-action-sheet-cancel" onClick={() => setRowSheet(null)}>
                Cancel
              </button>
            </div>
          </div>,
          document.body,
        )}
    </div>
  );
}
