import { create } from "zustand";
import type { Bookmark, Settings } from "../lib/types";
import { MAX_BOOKMARKS } from "../lib/types";
import { getSettings, updateSettings } from "../lib/commands";

interface SettingsState extends Settings {
  loadSettings: () => Promise<void>;
  /// Replace the entire bookmark list. Captures a full-settings snapshot
  /// before the optimistic write so a concurrent `loadSettings` mid-flight
  /// cannot pollute the payload sent to `update_settings`. Rolls back
  /// `bookmarks` on failure (e.g. server-side validation).
  setBookmarks: (next: Bookmark[]) => Promise<void>;
}

export const useSettingsStore = create<SettingsState>((set, get) => ({
  playbackMode: "directPlay",
  lookaheadDepth: 3,
  audioCacheLimitBytes: 2_147_483_648,
  imageCacheLimitBytes: 536_870_912,
  syncIntervalHours: 0,
  genreSource: "open",
  libraryPadding: 0,
  refuseHttp: false,
  lastSyncTimeSecs: 0,
  disableSpectrum: true,
  flatGenres: false,
  genreFuzzyThreshold: 0.8,
  eqEnabled: false,
  eqBands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  bookmarks: [],
  offlineMode: false,
  popularityDisplay: "hot",
  includePlexStyles: true,

  loadSettings: async () => {
    try {
      const s = await getSettings();
      set(s);
    } catch {
      // retain defaults
    }
  },

  setBookmarks: async (next: Bookmark[]) => {
    if (next.length > MAX_BOOKMARKS) {
      throw new Error(`Maximum ${MAX_BOOKMARKS} bookmarks.`);
    }
    const prev = get().bookmarks;
    const snapshot: Settings = { ...get(), bookmarks: next };
    set({ bookmarks: next });
    try {
      await updateSettings(snapshot);
    } catch (e) {
      set({ bookmarks: prev });
      throw e;
    }
  },
}));
