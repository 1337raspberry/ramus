import { create } from "zustand";
import type { Settings } from "../lib/types";
import { getSettings } from "../lib/commands";

interface SettingsState extends Settings {
  loadSettings: () => Promise<void>;
}

export const useSettingsStore = create<SettingsState>((set) => ({
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

  loadSettings: async () => {
    try {
      const s = await getSettings();
      set(s);
    } catch {
      // retain defaults
    }
  },
}));
