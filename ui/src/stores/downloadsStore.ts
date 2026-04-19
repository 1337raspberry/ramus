import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";

import {
  cancelAllDownloads,
  cancelDownload,
  downloadAlbum,
  downloadAllStarredAlbums,
  downloadAllStarredTracks,
  downloadTrack,
  estimateStarredAlbumsSize,
  estimateStarredTracksSize,
  getDownloadsOverview,
  removeAlbumDownloads,
  removeAllDownloads,
  removeDownload,
} from "../lib/commands";
import type { DownloadPhase, DownloadProgressPayload, DownloadsOverview } from "../lib/types";

/// Per-track state surfaced to components. Separate from the backend
/// `DownloadsOverview` so (...) menus can subscribe to individual rating
/// keys without thrashing.
export interface TrackDownloadState {
  phase: DownloadPhase;
  bytesWritten: number;
  totalBytes: number | null;
  error: string | null;
}

interface DownloadsState {
  overview: DownloadsOverview | null;
  /// Live per-track state for in-progress + recently-failed items. Completed
  /// items get pruned on the next overview refresh so the map doesn't grow
  /// unbounded. Keyed by rating key.
  trackState: Record<string, TrackDownloadState>;
  /// Source IDs of albums with ≥1 downloaded track, mirrored from
  /// `overview.albums` / `orphanTracks` for O(1) lookups from menus.
  downloadedAlbumIds: Set<string>;
  /// Rating keys of every downloaded track.
  downloadedTrackIds: Set<string>;
  /// Listener teardown set on first subscription.
  _listenersInstalled: boolean;

  refresh: () => Promise<void>;
  startTrackDownload: (ratingKey: string) => Promise<void>;
  startAlbumDownload: (albumRatingKey: string) => Promise<void>;
  startStarredTracks: () => Promise<number>;
  startStarredAlbums: () => Promise<number>;
  cancel: (ratingKey: string) => Promise<void>;
  cancelAll: () => Promise<void>;
  remove: (ratingKey: string) => Promise<void>;
  removeAlbum: (albumRatingKey: string) => Promise<void>;
  clearAll: () => Promise<void>;
  estimateStarredTracks: () => Promise<number>;
  estimateStarredAlbums: () => Promise<number>;
  ensureListeners: () => void;
}

export const useDownloadsStore = create<DownloadsState>((set, get) => ({
  overview: null,
  trackState: {},
  downloadedAlbumIds: new Set(),
  downloadedTrackIds: new Set(),
  _listenersInstalled: false,

  refresh: async () => {
    try {
      const overview = await getDownloadsOverview();
      const downloadedAlbumIds = new Set<string>();
      const downloadedTrackIds = new Set<string>();
      for (const a of overview.albums) downloadedAlbumIds.add(a.ratingKey);
      for (const t of overview.orphanTracks) {
        downloadedAlbumIds.add(t.albumRatingKey);
        downloadedTrackIds.add(t.ratingKey);
      }
      // Clear stale completed entries from trackState whenever we refresh.
      const prev = get().trackState;
      const next: Record<string, TrackDownloadState> = {};
      const activeQueue = new Set(overview.queue);
      const inProgress = overview.inProgress?.ratingKey;
      for (const [rk, st] of Object.entries(prev)) {
        if (st.phase === "done" || st.phase === "failed") continue;
        if (activeQueue.has(rk) || rk === inProgress) next[rk] = st;
      }
      if (overview.inProgress) {
        next[overview.inProgress.ratingKey] = {
          phase: "downloading",
          bytesWritten: overview.inProgress.bytesWritten,
          totalBytes: overview.inProgress.totalBytes,
          error: null,
        };
      }
      for (const rk of overview.queue) {
        if (!next[rk]) {
          next[rk] = {
            phase: "queued",
            bytesWritten: 0,
            totalBytes: null,
            error: null,
          };
        }
      }
      set({
        overview,
        trackState: next,
        downloadedAlbumIds,
        downloadedTrackIds,
      });
    } catch (e) {
      console.warn("downloadsStore.refresh failed", e);
    }
  },

  startTrackDownload: async (ratingKey) => {
    // Optimistic: mark queued immediately so the (...) menu flips state
    // without waiting for the roundtrip + events.
    set((s) => ({
      trackState: {
        ...s.trackState,
        [ratingKey]: {
          phase: "queued",
          bytesWritten: 0,
          totalBytes: null,
          error: null,
        },
      },
    }));
    try {
      await downloadTrack(ratingKey);
    } catch (e) {
      set((s) => {
        const next = { ...s.trackState };
        delete next[ratingKey];
        return { trackState: next };
      });
      throw e;
    }
  },

  startAlbumDownload: async (albumRatingKey) => {
    await downloadAlbum(albumRatingKey);
    await get().refresh();
  },

  startStarredTracks: async () => {
    const n = await downloadAllStarredTracks();
    await get().refresh();
    return n;
  },

  startStarredAlbums: async () => {
    const n = await downloadAllStarredAlbums();
    await get().refresh();
    return n;
  },

  cancel: async (ratingKey) => {
    await cancelDownload(ratingKey);
    set((s) => {
      const next = { ...s.trackState };
      delete next[ratingKey];
      return { trackState: next };
    });
    await get().refresh();
  },

  cancelAll: async () => {
    await cancelAllDownloads();
    set({ trackState: {} });
    await get().refresh();
  },

  remove: async (ratingKey) => {
    await removeDownload(ratingKey);
    await get().refresh();
  },

  removeAlbum: async (albumRatingKey) => {
    await removeAlbumDownloads(albumRatingKey);
    await get().refresh();
  },

  clearAll: async () => {
    await removeAllDownloads();
    set({ trackState: {} });
    await get().refresh();
  },

  estimateStarredTracks: async () => {
    return await estimateStarredTracksSize();
  },

  estimateStarredAlbums: async () => {
    return await estimateStarredAlbumsSize();
  },

  ensureListeners: () => {
    if (get()._listenersInstalled) return;
    set({ _listenersInstalled: true });

    listen<DownloadProgressPayload>("download-progress", (event) => {
      const p = event.payload;
      set((s) => ({
        trackState: {
          ...s.trackState,
          [p.ratingKey]: {
            phase: p.phase,
            bytesWritten: p.bytesWritten,
            totalBytes: p.totalBytes,
            error: p.error,
          },
        },
      }));
    });

    listen("downloads-changed", () => {
      // Small debounce via microtask — multiple emits in a tick coalesce.
      Promise.resolve().then(() => get().refresh());
    });
  },
}));

/// Convenience selectors.
export const selectIsDownloaded = (ratingKey: string) => (s: DownloadsState) =>
  s.downloadedTrackIds.has(ratingKey);

export const selectTrackDownloadState = (ratingKey: string) => (s: DownloadsState) =>
  s.trackState[ratingKey];

export const selectIsAlbumDownloaded = (albumRatingKey: string) => (s: DownloadsState) =>
  s.downloadedAlbumIds.has(albumRatingKey);
