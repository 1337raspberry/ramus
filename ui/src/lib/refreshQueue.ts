import { getQueue } from "./commands";
import { usePlaybackStore } from "../stores/playbackStore";

/**
 * Pull the authoritative queue from the backend into `playbackStore`.
 *
 * The store's copy is otherwise refreshed only on a track change, so a queue
 * mutation issued straight through the IPC layer (append, insert-next) has to
 * call this — without it the Up Next list keeps rendering the pre-mutation
 * queue until the next track boundary, which reads as the mutation having
 * silently failed.
 *
 * Errors are swallowed on purpose: the mutation itself has already landed
 * server-side, and the list self-heals at the next track change.
 */
export async function refreshQueue(): Promise<void> {
  try {
    const queue = await getQueue();
    usePlaybackStore.setState({ queue });
  } catch {
    /* keep the stale list rather than blanking it */
  }
}
