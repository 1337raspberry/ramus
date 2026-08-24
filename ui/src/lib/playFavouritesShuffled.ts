import { getFavouriteTracks, playTracks } from "./commands";
import { shuffleTracks } from "./shuffle";
import { useConnectionStore } from "../stores/connectionStore";
import { useDownloadsStore } from "../stores/downloadsStore";
import { useToastStore } from "../components/Toast";

/**
 * Replace the queue with every favourite track, artist-stratified shuffled,
 * and start playback immediately. Offline, only downloaded favourites are
 * eligible — a track that can't resolve a URL would silently stall mpv.
 */
export async function playFavouritesShuffled() {
  const toast = useToastStore.getState().show;

  let tracks;
  try {
    tracks = await getFavouriteTracks();
  } catch {
    toast("Couldn't load favourites");
    return;
  }

  if (useConnectionStore.getState().effectiveOffline) {
    const downloaded = useDownloadsStore.getState().downloadedTrackIds;
    tracks = tracks.filter((t) => downloaded.has(t.ratingKey));
  }

  if (tracks.length === 0) {
    toast("No favourite tracks");
    return;
  }

  try {
    await playTracks(shuffleTracks(tracks), 0);
    const noun = tracks.length === 1 ? "favourite" : "favourites";
    toast(`Shuffling ${tracks.length} ${noun}`);
  } catch {
    toast("Couldn't start playback");
  }
}
