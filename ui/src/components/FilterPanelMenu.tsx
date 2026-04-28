import { useEffect, useRef, useState } from "react";
import { IconMoreDots } from "./Icons";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { useConnectionStore } from "../stores/connectionStore";
import { useDownloadsStore } from "../stores/downloadsStore";
import { useToastStore } from "./Toast";
import { appendToQueue, getTracksForAlbum, playTracks } from "../lib/commands";
import type { Album, Track } from "../lib/types";

interface Props {
  /** Optional close-handler so the parent panel can dismiss after an action
   * (e.g. close the desktop filter dropdown after queuing). */
  onAfterAction?: () => void;
}

// In-place Fisher–Yates. Used for the queue actions which all default to
// shuffled output — for the "Albums" mode the shuffle is applied to the
// album order (tracks stay sequential within each album so full LPs play
// in their original sequence); for the "Tracks" mode the shuffle is
// applied to the flattened track list.
function shuffleInPlace<T>(arr: T[]): T[] {
  for (let i = arr.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [arr[i], arr[j]] = [arr[j], arr[i]];
  }
  return arr;
}

type QueueMode = "albums" | "tracks";

/**
 * Build a queue from a set of albums and either start playback (when nothing
 * is playing) or append to the existing queue. Always shuffles by default —
 * "albums" mode shuffles whole-album order, "tracks" mode shuffles the
 * flattened track list. `trackFilter` narrows tracks before shuffle.
 */
async function queueAlbumsAsTracks(
  albums: Album[],
  mode: QueueMode,
  trackFilter: ((t: Track) => boolean) | null,
) {
  if (albums.length === 0) {
    useToastStore.getState().show("No albums to queue");
    return;
  }

  // Shuffle album order up front for "albums" mode so even partial fetch
  // failures still produce randomised whole-album playback. Tracks stay
  // sequential within each album (Fisher-Yates is applied to the album
  // array, not the flattened tracks).
  const orderedAlbums = mode === "albums" ? shuffleInPlace(albums.slice()) : albums;

  // Fetch tracks sequentially — Plex kills concurrent remote downloads, and
  // even on cache hits the local SQLite mutex would serialise these anyway.
  // `Promise.all` was only marginally faster on local-only setups and made
  // remote-server users hit transcode rate limits.
  const trackLists: Track[][] = [];
  for (const a of orderedAlbums) {
    try {
      trackLists.push(await getTracksForAlbum(a.ratingKey));
    } catch {
      trackLists.push([]);
    }
  }
  let tracks = trackLists.flat();
  if (trackFilter) tracks = tracks.filter(trackFilter);

  // Offline guard: drop tracks that aren't in the persistent download set.
  // Mirrors `libraryStore.playAlbum` — without this, mpv receives null URLs
  // and silently stalls.
  if (useConnectionStore.getState().effectiveOffline) {
    const downloaded = useDownloadsStore.getState().downloadedTrackIds;
    tracks = tracks.filter((t) => downloaded.has(t.ratingKey));
  }

  if (tracks.length === 0) {
    useToastStore.getState().show("No tracks to queue");
    return;
  }

  if (mode === "tracks") shuffleInPlace(tracks);

  const playing = !!usePlaybackStore.getState().currentTrack;
  const noun = tracks.length === 1 ? "track" : "tracks";
  try {
    if (playing) {
      await appendToQueue(tracks);
      useToastStore.getState().show(`Added ${tracks.length} ${noun} to queue`);
    } else {
      await playTracks(tracks, 0);
      useToastStore.getState().show(`Playing ${tracks.length} ${noun}`);
    }
  } catch {
    useToastStore.getState().show("Couldn't queue tracks");
  }
}

export default function FilterPanelMenu({ onAfterAction }: Props) {
  const albums = useLibraryStore((s) => s.albums);
  const favouriteTracksFilterOn = useLibraryStore((s) => s.albumFilters.favouriteTracks);
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    // pointerdown rather than mousedown so iOS taps register — mousedown is
    // only synthesised after the touchend, which is too late.
    const handler = (e: Event) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("pointerdown", handler);
    return () => document.removeEventListener("pointerdown", handler);
  }, [open]);

  const close = () => {
    setOpen(false);
    onAfterAction?.();
  };

  const handleAddAlbums = async () => {
    close();
    await queueAlbumsAsTracks(albums, "albums", null);
  };

  const handleAddTracks = async () => {
    close();
    // Respect the favourite-tracks filter at the track level — if it's on,
    // only queue starred tracks. Otherwise we still take every track of every
    // visible album, but shuffled flat instead of preserving album order.
    const filter = favouriteTracksFilterOn ? (t: Track) => t.isFavourite : null;
    await queueAlbumsAsTracks(albums, "tracks", filter);
  };

  const handleBookmark = () => {
    close();
    useToastStore.getState().show("Bookmarks coming soon");
  };

  return (
    <div className="filter-panel-menu" ref={wrapRef}>
      <button
        type="button"
        className="filter-panel-menu-btn"
        aria-label="More actions"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((v) => !v);
        }}
      >
        <IconMoreDots size={22} />
      </button>
      {open && (
        <div className="filter-panel-menu-dropdown" role="menu">
          <button type="button" role="menuitem" onClick={handleAddAlbums}>
            Add all Albums to playlist
          </button>
          <button type="button" role="menuitem" onClick={handleAddTracks}>
            Add all tracks to playlist
          </button>
          <button type="button" role="menuitem" onClick={handleBookmark}>
            Bookmark
          </button>
        </div>
      )}
    </div>
  );
}
