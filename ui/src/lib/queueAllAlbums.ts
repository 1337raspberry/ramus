import { appendToQueue, getTracksForAlbum } from "./commands";
import { useConnectionStore } from "../stores/connectionStore";
import { useDownloadsStore } from "../stores/downloadsStore";
import { useToastStore } from "../components/Toast";
import type { Album, Track } from "./types";

/**
 * Append every given album's tracks to the end of the now-playing queue.
 * Albums keep their displayed order and tracks stay sequential within each
 * album — ordering is the sort chrome's job (Random sort covers the
 * shuffled case), so this deliberately adds exactly what the grid shows.
 * Always appends, never starts playback: with nothing playing the tracks
 * simply become the queue, paused.
 */
export async function appendAlbumsToQueue(albums: Album[]) {
  if (albums.length === 0) {
    useToastStore.getState().show("No albums to queue");
    return;
  }

  // Fetch tracks sequentially — Plex kills concurrent remote downloads, and
  // even on cache hits the local SQLite mutex would serialise these anyway.
  const trackLists: Track[][] = [];
  for (const a of albums) {
    try {
      trackLists.push(await getTracksForAlbum(a.ratingKey));
    } catch {
      trackLists.push([]);
    }
  }
  let tracks = trackLists.flat();

  // Offline guard: drop tracks that aren't in the persistent download set.
  // Without this, mpv receives null URLs and silently stalls.
  if (useConnectionStore.getState().effectiveOffline) {
    const downloaded = useDownloadsStore.getState().downloadedTrackIds;
    tracks = tracks.filter((t) => downloaded.has(t.ratingKey));
  }

  if (tracks.length === 0) {
    useToastStore.getState().show("No tracks to queue");
    return;
  }

  try {
    await appendToQueue(tracks);
    const noun = tracks.length === 1 ? "track" : "tracks";
    useToastStore.getState().show(`Added ${tracks.length} ${noun} to queue`);
  } catch {
    useToastStore.getState().show("Couldn't queue tracks");
  }
}
