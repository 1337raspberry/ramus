import { create } from "zustand";
import type { SavedSearch, Settings } from "../lib/types";
import { MAX_SAVED_SEARCHES } from "../lib/types";
import { getSettings, updateSettings } from "../lib/commands";

interface SettingsState extends Settings {
  loadSettings: () => Promise<void>;
  /// Replace the entire saved-search list. Captures a full-settings
  /// snapshot before the optimistic write so a concurrent `loadSettings`
  /// mid-flight cannot pollute the payload sent to `update_settings`.
  /// Rolls back `savedSearches` on failure (e.g. server-side validation).
  setSavedSearches: (next: SavedSearch[]) => Promise<void>;
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
  disableSpectrum: false,
  flatGenres: false,
  eqEnabled: false,
  eqBands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  savedSearches: [],
  offlineMode: false,
  popularityDisplay: "hot",

  loadSettings: async () => {
    try {
      const s = await getSettings();
      set(s);
    } catch {
      // retain defaults
    }
  },

  setSavedSearches: async (next: SavedSearch[]) => {
    if (next.length > MAX_SAVED_SEARCHES) {
      throw new Error(`Maximum ${MAX_SAVED_SEARCHES} saved searches.`);
    }
    const prev = get().savedSearches;
    const snapshot: Settings = { ...get(), savedSearches: next };
    set({ savedSearches: next });
    try {
      await updateSettings(snapshot);
    } catch (e) {
      set({ savedSearches: prev });
      throw e;
    }
  },
}));
