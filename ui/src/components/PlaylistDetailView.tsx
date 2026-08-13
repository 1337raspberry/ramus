import { useEffect, useRef, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { useToastStore } from "./Toast";
import type { PlaylistItem } from "../lib/types";
import {
  ART_SIZE,
  appendToQueue,
  deletePlaylist,
  getPlaylistItems,
  movePlaylistItem,
  playTracks,
  removePlaylistItem,
} from "../lib/commands";
import { useListReorder } from "../lib/useListReorder";
import { useArtUrl } from "../lib/useArtUrl";
import { shuffleTracks } from "../lib/shuffle";
import { formatDuration, formatLongDuration } from "../lib/format";
import { IconClose, IconMusicNote, IconPlay, IconShuffle } from "./Icons";

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
 * entry; the `::` handle drags to reorder (regular playlists only).
 */
export default function PlaylistDetailView() {
  const playlist = useLibraryStore((s) => s.browsePlaylist);
  const [items, setItems] = useState<PlaylistItem[] | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const confirmTimer = useRef<number | null>(null);
  const { artSrc, artErr, setArtErr } = useArtUrl(playlist?.thumb, ART_SIZE.MEDIUM);

  useEffect(() => {
    setItems(null);
    setConfirmDelete(false);
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

  const handleDelete = () => {
    if (!confirmDelete) {
      setConfirmDelete(true);
      confirmTimer.current = window.setTimeout(() => setConfirmDelete(false), 3000);
      return;
    }
    deletePlaylist(playlist.sourceId)
      .then(() => {
        useToastStore.getState().show(`Deleted “${playlist.title}”`);
        useLibraryStore.setState({ browsePlaylist: null });
      })
      .catch(() => useToastStore.getState().show("Couldn't delete playlist"));
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
          <button
            className={`playlist-view-btn danger${confirmDelete ? " confirm" : ""}`}
            onClick={handleDelete}
          >
            {confirmDelete ? "Confirm delete?" : "Delete"}
          </button>
        </div>
      </div>

      {items === null ? (
        <div className="empty-state">Loading…</div>
      ) : items.length === 0 ? (
        <div className="empty-state">This playlist is empty</div>
      ) : (
        <div className="playlist-scroll">
          {items.map((item, index) => (
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
                <span className="playlist-row-duration">{formatDuration(item.track.duration)}</span>
              </button>
              <button
                className="playlist-row-queue"
                title="Add to queue"
                onClick={() => appendToQueue([item.track]).catch(() => {})}
              >
                +
              </button>
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
          ))}
        </div>
      )}
    </div>
  );
}
