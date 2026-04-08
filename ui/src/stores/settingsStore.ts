import { create } from "zustand";
import type { Settings } from "../lib/types";
import { getSettings } from "../lib/commands";

interface SettingsState extends Settings {
  loadSettings: () => Promise<void>;
}

export const useSettingsStore = create<SettingsState>((set) => ({
  // Defaults (overwritten once loadSettings completes)
  playbackMode: "directPlay",
  lookaheadDepth: 3,
  audioCacheLimitBytes: 2_147_483_648,
  syncIntervalHours: 0,
  genreSource: "open",
  libraryPadding: 0,
  refuseHttp: false,
  lastSyncTimeSecs: 0,

  loadSettings: async () => {
    try {
      const s = await getSettings();
      set(s);
    } catch {
      // keep defaults
    }
  },
}));
