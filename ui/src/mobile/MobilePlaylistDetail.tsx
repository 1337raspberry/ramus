import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { Playlist, PlaylistItem } from "../lib/types";
import {
  deletePlaylist,
  getPlaylistItems,
  movePlaylistItem,
  playTracks,
  removePlaylistItem,
  appendToQueue,
} from "../lib/commands";
import { useListReorder } from "../lib/useListReorder";
import { useLongPress } from "../lib/useLongPress";
import { useToastStore } from "../components/Toast";
import { pushBackHandler } from "../lib/backHandler";
import { formatDuration } from "../lib/format";
import { IconChevronLeft, IconMoreDots } from "../components/Icons";

const ROW_HEIGHT = 56;

interface Props {
  playlist: Playlist;
  onBack: () => void;
}

/** One row's body: tap plays the playlist from here, long-press opens the
 * per-row sheet. Isolated so the long-press hook runs per row. */
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
  return (
    <button className="playlist-row-body" {...longPress}>
      <div className="playlist-row-info">
        <div className="playlist-row-title">{item.track.title}</div>
        <div className="playlist-row-artist">{item.track.trackArtist || item.track.artistName}</div>
      </div>
      <span className="playlist-row-duration">{formatDuration(item.track.duration)}</span>
    </button>
  );
}

/**
 * Playlist detail: ordered track list with drag-to-reorder via the `::` grab
 * handles (regular playlists only — smart playlists are display/play only).
 * Reorder is optimistic; the server's refreshed list reconciles on landing.
 */
export default function MobilePlaylistDetail({ playlist, onBack }: Props) {
  const [items, setItems] = useState<PlaylistItem[] | null>(null);
  const [showMenu, setShowMenu] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [rowSheet, setRowSheet] = useState<number | null>(null);

  useEffect(() => {
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
  }, [playlist.sourceId]);

  useEffect(() => {
    if (!showMenu && !confirmDelete && rowSheet === null) return;
    return pushBackHandler(() => {
      setShowMenu(false);
      setConfirmDelete(false);
      setRowSheet(null);
      return true;
    });
  }, [showMenu, confirmDelete, rowSheet]);

  const reorder = (from: number, to: number) => {
    if (!items) return;
    const next = [...items];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    setItems(next);
    const afterItemId = to > 0 ? next[to - 1].playlistItemId : null;
    movePlaylistItem(playlist.sourceId, moved.playlistItemId, afterItemId)
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

  const play = (startAt: number) => {
    if (!items || items.length === 0) return;
    playTracks(
      items.map((i) => i.track),
      startAt,
    ).catch(() => {});
  };

  const shuffle = () => {
    if (!items || items.length === 0) return;
    const tracks = items.map((i) => i.track);
    for (let i = tracks.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [tracks[i], tracks[j]] = [tracks[j], tracks[i]];
    }
    playTracks(tracks, 0).catch(() => {});
  };

  const removeRow = (index: number) => {
    if (!items) return;
    const item = items[index];
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

  const handleDelete = () => {
    deletePlaylist(playlist.sourceId)
      .then(() => {
        useToastStore.getState().show(`Deleted “${playlist.title}”`);
        onBack();
      })
      .catch(() => useToastStore.getState().show("Couldn't delete playlist"));
  };

  const subtitleBits: string[] = [];
  const count = items?.length ?? playlist.trackCount;
  if (count != null) subtitleBits.push(`${count} track${count === 1 ? "" : "s"}`);
  if (playlist.duration) subtitleBits.push(formatDuration(playlist.duration));
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

      {subtitleBits.length > 0 && (
        <div className="playlist-subtitle">{subtitleBits.join(" · ")}</div>
      )}

      {items === null ? (
        <div className="mobile-empty">Loading…</div>
      ) : items.length === 0 ? (
        <div className="mobile-empty">This playlist is empty</div>
      ) : (
        <div className="playlist-scroll">
          {items.map((item, index) => (
            <div
              key={item.playlistItemId}
              className="playlist-row"
              ref={(el) => setRowRef(index, el)}
            >
              <RowBody item={item} onPlay={() => play(index)} onSheet={() => setRowSheet(index)} />
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
          ))}
        </div>
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
                    play(0);
                  }}
                >
                  Play
                </button>
                <button
                  onClick={() => {
                    setShowMenu(false);
                    shuffle();
                  }}
                >
                  Shuffle
                </button>
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
                    appendToQueue([track]).catch(() => {});
                  }}
                >
                  Add to Queue
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
