import { useCallback } from "react";
import { appendToQueue, getQueue, getTracksForAlbum, insertNext } from "./commands";
import { usePlaybackStore } from "../stores/playbackStore";

/// Returns a single callback that queues every track of an album either at
/// the head of the up-next list or at the tail. Shared by the desktop album
/// card's (...) menu and the mobile long-press action sheet; the underlying
/// flow (fetch tracks → call IPC → re-read queue) is identical in both.
export function useQueueAlbum(ratingKey: string): (mode: "next" | "append") => Promise<void> {
  return useCallback(
    async (mode) => {
      const tracks = await getTracksForAlbum(ratingKey);
      const fn = mode === "next" ? insertNext : appendToQueue;
      await fn(tracks);
      const q = await getQueue();
      usePlaybackStore.setState({ queue: q });
    },
    [ratingKey],
  );
}
