import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { Playlist } from "../lib/types";
import { addTracksToPlaylist, createPlaylist, getPlaylists } from "../lib/commands";
import { useToastStore } from "../components/Toast";
import { pushBackHandler } from "../lib/backHandler";

interface Props {
  /** Resolved lazily so album flows can fetch their track list on demand. */
  getTrackIds: () => Promise<string[]>;
  /** Sheet heading, e.g. the track or album title being added. */
  heading: string;
  /** Skip the picker and go straight to naming a new playlist
   * ("Save Queue as Playlist…"). */
  createOnly?: boolean;
  /** Layer above the now-playing sheet (z-1100) when opened from it. */
  overSheet?: boolean;
  onDismiss: () => void;
}

/**
 * Action-sheet picker for adding tracks to a playlist. Lists the account's
 * regular playlists (smart ones are filter-driven and can't take items) plus
 * a "New Playlist…" row that flips the sheet into name entry.
 */
export default function PlaylistPickerSheet({
  getTrackIds,
  heading,
  createOnly,
  overSheet,
  onDismiss,
}: Props) {
  const [playlists, setPlaylists] = useState<Playlist[] | null>(null);
  const [naming, setNaming] = useState(!!createOnly);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (createOnly) return;
    let cancelled = false;
    getPlaylists()
      .then((list) => {
        if (!cancelled) setPlaylists(list.filter((p) => !p.smart));
      })
      .catch(() => {
        if (!cancelled) setPlaylists([]);
      });
    return () => {
      cancelled = true;
    };
  }, [createOnly]);

  useEffect(
    () =>
      pushBackHandler(() => {
        onDismiss();
        return true;
      }),
    [onDismiss],
  );

  const addTo = (playlist: Playlist) => {
    if (busy) return;
    setBusy(true);
    getTrackIds()
      .then((ids) => addTracksToPlaylist(playlist.sourceId, ids))
      .then(() => {
        useToastStore.getState().show(`Added to “${playlist.title}”`);
        onDismiss();
      })
      .catch(() => {
        useToastStore.getState().show("Couldn't add to playlist");
        setBusy(false);
      });
  };

  const createNew = () => {
    const trimmed = name.trim();
    if (!trimmed || busy) return;
    setBusy(true);
    getTrackIds()
      .then((ids) => createPlaylist(trimmed, ids))
      .then((p) => {
        useToastStore.getState().show(`Created “${p.title}”`);
        onDismiss();
      })
      .catch(() => {
        useToastStore.getState().show("Couldn't create playlist");
        setBusy(false);
      });
  };

  return createPortal(
    <div
      className={`mobile-action-sheet-backdrop${overSheet ? " over-sheet" : ""}`}
      onClick={(e) => {
        if (e.target === e.currentTarget) onDismiss();
      }}
    >
      <div className="mobile-action-sheet">
        <div className="mobile-action-sheet-group">
          <div className="mobile-action-sheet-header">
            {naming ? "Name the new playlist" : `Add “${heading}” to a playlist`}
          </div>
          {naming ? (
            <div className="playlist-name-entry">
              <input
                className="playlist-name-input"
                type="text"
                value={name}
                autoFocus
                placeholder="Playlist name"
                onChange={(e) => setName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") createNew();
                }}
              />
              <button
                className="playlist-name-create"
                disabled={!name.trim() || busy}
                onClick={createNew}
              >
                Create
              </button>
            </div>
          ) : (
            <div className="mobile-collection-list">
              {playlists === null ? (
                <div className="mobile-collection-empty">Loading…</div>
              ) : (
                <>
                  {playlists.map((p) => (
                    <button key={p.sourceId} disabled={busy} onClick={() => addTo(p)}>
                      <span className="mobile-collection-name">{p.title}</span>
                      {p.trackCount != null && (
                        <span className="mobile-lists-row-count">{p.trackCount}</span>
                      )}
                    </button>
                  ))}
                  <button disabled={busy} onClick={() => setNaming(true)}>
                    <span className="mobile-collection-name">New Playlist…</span>
                  </button>
                </>
              )}
            </div>
          )}
        </div>
        <button className="mobile-action-sheet-cancel" onClick={onDismiss}>
          Cancel
        </button>
      </div>
    </div>,
    document.body,
  );
}
