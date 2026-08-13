import { useEffect, useState } from "react";
import type { Playlist } from "../lib/types";
import { addTracksToPlaylist, createPlaylist, getPlaylists } from "../lib/commands";
import { useToastStore } from "./Toast";
import { useLibraryStore } from "../stores/libraryStore";

interface Props {
  /** Resolved lazily so album flows can fetch their track list on demand. */
  getTrackIds: () => Promise<string[]>;
  /** Skip the picker and go straight to naming a new playlist
   * ("Save Queue as Playlist…"). */
  createOnly?: boolean;
  /** Called after a successful add/create — the shell dismisses itself. */
  onDone: () => void;
  /** Fired when the body flips between the picker list and the name entry,
   * so the shell can update its header copy (the mobile sheet also lifts
   * itself above the software keyboard). */
  onNamingChange?: (naming: boolean) => void;
}

/**
 * Body for adding tracks to a playlist, shared between the mobile action
 * sheet and the desktop modal. Lists the account's regular playlists (smart
 * ones are filter-driven and can't take items) plus a "New Playlist…" row
 * that flips into name entry.
 */
export default function PlaylistPickerContent({
  getTrackIds,
  createOnly,
  onDone,
  onNamingChange,
}: Props) {
  const [playlists, setPlaylists] = useState<Playlist[] | null>(null);
  // Distinct from an empty list: a failed fetch coerced to [] would read as
  // "you have no playlists", which is a different problem entirely. Creating
  // stays available either way — it's the one action that doesn't need the
  // existing list to have loaded.
  const [loadFailed, setLoadFailed] = useState(false);
  const [naming, setNamingState] = useState(!!createOnly);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);

  const setNaming = (next: boolean) => {
    setNamingState(next);
    onNamingChange?.(next);
  };

  useEffect(() => {
    if (createOnly) return;
    let cancelled = false;
    getPlaylists()
      .then((list) => {
        if (!cancelled) setPlaylists(list.filter((p) => !p.smart));
      })
      .catch(() => {
        if (cancelled) return;
        setPlaylists([]);
        setLoadFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [createOnly]);

  const addTo = (playlist: Playlist) => {
    if (busy) return;
    setBusy(true);
    getTrackIds()
      .then((ids) => addTracksToPlaylist(playlist.sourceId, ids))
      .then(() => {
        useToastStore.getState().show(`Added to “${playlist.title}”`);
        onDone();
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
        useLibraryStore.getState().bumpPlaylistsRevision();
        onDone();
      })
      .catch(() => {
        useToastStore.getState().show("Couldn't create playlist");
        setBusy(false);
      });
  };

  if (naming) {
    return (
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
    );
  }

  return (
    <div className="mobile-collection-list">
      {playlists === null ? (
        <div className="mobile-collection-empty">Loading…</div>
      ) : (
        <>
          {loadFailed && (
            <div className="mobile-collection-empty">Couldn&rsquo;t load your playlists</div>
          )}
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
  );
}
