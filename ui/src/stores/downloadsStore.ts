import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";

import {
  cancelAllDownloads,
  cancelDownload,
  downloadAlbum,
  downloadAllStarredAlbums,
  downloadAllStarredTracks,
  downloadSearchResults,
  downloadTrack,
  estimateSearchSize,
  estimateStarredAlbumsSize,
  estimateStarredTracksSize,
  getDownloadsOverview,
  removeAlbumDownloads,
  removeAllDownloads,
  removeDownload,
} from "../lib/commands";
import type {
  DownloadProgressPayload,
  DownloadsOverview,
  SearchDownloadEstimate,
} from "../lib/types";

/// Live per-track byte progress. Kept for the currently-in-flight item
/// only — once a track finishes (`done` / `failed`) we drop it from this
/// map so the rest of the UI reads from `overview` instead.
interface LiveProgress {
  bytesWritten: number;
  totalBytes: number | null;
}

interface DownloadsState {
  overview: DownloadsOverview | null;
  /// Currently-downloading byte progress, updated by the download-progress
  /// event stream. Separate from `overview.inProgress` because the backend
  /// only snapshots that at overview time — this map stays live between
  /// overview refreshes.
  liveProgress: LiveProgress | null;
  /// Per-track status optimistically set by user actions. Used by the
  /// (...) menus to flip "Download" → "Downloading…" / "Remove Download"
  /// without waiting for an event round-trip.
  trackPhase: Record<string, "queued" | "downloading">;
  /// Source IDs of albums with ≥1 downloaded track, mirrored from
  /// `overview.albums` / `orphanTracks` for O(1) lookups from menus.
  downloadedAlbumIds: Set<string>;
  /// Rating keys of every downloaded track.
  downloadedTrackIds: Set<string>;
  /// Listener teardown guard.
  _listenersInstalled: boolean;
  /// Pending refresh handle so we coalesce bursts of downloads-changed.
  _refreshTimer: ReturnType<typeof setTimeout> | null;

  refresh: () => Promise<void>;
  scheduleRefresh: () => void;
  startTrackDownload: (ratingKey: string) => Promise<void>;
  startAlbumDownload: (albumRatingKey: string) => Promise<void>;
  startStarredTracks: () => Promise<number>;
  startStarredAlbums: () => Promise<number>;
  startSavedSearchDownload: (query: string) => Promise<number>;
  estimateSavedSearch: (query: string) => Promise<SearchDownloadEstimate>;
  cancel: (ratingKey: string) => Promise<void>;
  cancelAll: () => Promise<void>;
  remove: (ratingKey: string) => Promise<void>;
  removeAlbum: (albumRatingKey: string) => Promise<void>;
  clearAll: () => Promise<void>;
  estimateStarredTracks: () => Promise<number>;
  estimateStarredAlbums: () => Promise<number>;
  ensureListeners: () => void;
}

/// Refresh no more than once every 400ms under event bursts.
const REFRESH_DEBOUNCE_MS = 400;

export const useDownloadsStore = create<DownloadsState>((set, get) => ({
  overview: null,
  liveProgress: null,
  trackPhase: {},
  downloadedAlbumIds: new Set(),
  downloadedTrackIds: new Set(),
  _listenersInstalled: false,
  _refreshTimer: null,

  refresh: async () => {
    try {
      const overview = await getDownloadsOverview();
      const downloadedAlbumIds = new Set<string>();
      for (const a of overview.albums) downloadedAlbumIds.add(a.ratingKey);
      for (const t of overview.orphanTracks) {
        downloadedAlbumIds.add(t.albumRatingKey);
      }
      // Use the flat rating-key list from the backend — orphanTracks
      // only covers tracks whose album has exactly one download, which
      // misses every track on a multi-track-downloaded album.
      const downloadedTrackIds = new Set<string>(overview.downloadedRatingKeys);
      // Strip optimistic phases for items no longer pending (either
      // finished or cancelled — neither is in overview.queue / inProgress).
      const prev = get().trackPhase;
      const activeQueue = new Set(overview.queue);
      const inFlight = overview.inProgress?.ratingKey;
      const nextPhase: Record<string, "queued" | "downloading"> = {};
      for (const [rk, p] of Object.entries(prev)) {
        if (activeQueue.has(rk) || rk === inFlight) nextPhase[rk] = p;
      }
      // If the backend says something's in-flight, trust it over any
      // stale optimistic "queued" we might be holding.
      if (inFlight) nextPhase[inFlight] = "downloading";
      set({
        overview,
        trackPhase: nextPhase,
        downloadedAlbumIds,
        downloadedTrackIds,
        // Sync liveProgress with the backend snapshot on refresh. Between
        // refreshes, the download-progress listener keeps it current.
        liveProgress: overview.inProgress
          ? {
              bytesWritten: overview.inProgress.bytesWritten,
              totalBytes: overview.inProgress.totalBytes,
            }
          : null,
      });
    } catch (e) {
      console.warn("downloadsStore.refresh failed", e);
    }
  },

  scheduleRefresh: () => {
    const existing = get()._refreshTimer;
    if (existing) return;
    const t = setTimeout(() => {
      set({ _refreshTimer: null });
      void get().refresh();
    }, REFRESH_DEBOUNCE_MS);
    set({ _refreshTimer: t });
  },

  startTrackDownload: async (ratingKey) => {
    set((s) => ({
      trackPhase: { ...s.trackPhase, [ratingKey]: "queued" },
    }));
    try {
      await downloadTrack(ratingKey);
      get().scheduleRefresh();
    } catch (e) {
      set((s) => {
        const next = { ...s.trackPhase };
        delete next[ratingKey];
        return { trackPhase: next };
      });
      throw e;
    }
  },

  startAlbumDownload: async (albumRatingKey) => {
    await downloadAlbum(albumRatingKey);
    get().scheduleRefresh();
  },

  startStarredTracks: async () => {
    const n = await downloadAllStarredTracks();
    get().scheduleRefresh();
    return n;
  },

  startStarredAlbums: async () => {
    const n = await downloadAllStarredAlbums();
    get().scheduleRefresh();
    return n;
  },

  startSavedSearchDownload: async (query) => {
    const n = await downloadSearchResults(query);
    get().scheduleRefresh();
    return n;
  },

  estimateSavedSearch: async (query) => {
    return await estimateSearchSize(query);
  },

  cancel: async (ratingKey) => {
    await cancelDownload(ratingKey);
    set((s) => {
      const next = { ...s.trackPhase };
      delete next[ratingKey];
      return { trackPhase: next };
    });
    get().scheduleRefresh();
  },

  cancelAll: async () => {
    await cancelAllDownloads();
    set({ trackPhase: {}, liveProgress: null });
    get().scheduleRefresh();
  },

  remove: async (ratingKey) => {
    await removeDownload(ratingKey);
    get().scheduleRefresh();
  },

  removeAlbum: async (albumRatingKey) => {
    await removeAlbumDownloads(albumRatingKey);
    get().scheduleRefresh();
  },

  clearAll: async () => {
    await removeAllDownloads();
    set({ trackPhase: {}, liveProgress: null });
    get().scheduleRefresh();
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
      if (p.phase === "downloading") {
        set({
          liveProgress: {
            bytesWritten: p.bytesWritten,
            totalBytes: p.totalBytes,
          },
        });
      } else if (p.phase === "done" || p.phase === "failed") {
        // Terminal — drop any optimistic phase and clear live progress
        // (the next start will set it again).
        set((s) => {
          const next = { ...s.trackPhase };
          delete next[p.ratingKey];
          return { trackPhase: next, liveProgress: null };
        });
      }
    });

    listen("downloads-changed", () => {
      get().scheduleRefresh();
    });
  },
}));

/// Convenience selectors.
export const selectIsDownloaded = (ratingKey: string) => (s: DownloadsState) =>
  s.downloadedTrackIds.has(ratingKey);

export const selectIsAlbumDownloaded = (albumRatingKey: string) => (s: DownloadsState) =>
  s.downloadedAlbumIds.has(albumRatingKey);

/// Returns `null` when the track isn't currently queued or downloading.
/// Consumers use this to flip menu labels without re-subscribing to the
/// full track map on every progress event.
export const selectTrackPhase =
  (ratingKey: string) =>
  (s: DownloadsState): "queued" | "downloading" | null =>
    s.trackPhase[ratingKey] ?? null;
