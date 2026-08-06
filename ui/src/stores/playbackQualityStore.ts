import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";

import type { PlaybackQualityPayload } from "../lib/types";

/// Whether the current connection can sustain what's playing, and what the
/// backend has done about it. Mirrors `connectionStore`'s shape — wires its
/// Tauri subscription once via `ensureListener` and is readable from
/// non-React code through `getState()`.
///
/// The backend emits only on change, so no local debouncing is needed.
interface PlaybackQualityState extends PlaybackQualityPayload {
  _listenerInstalled: boolean;
  ensureListener: () => void;
}

export const usePlaybackQualityStore = create<PlaybackQualityState>((set, get) => ({
  starving: false,
  degradedToKbps: null,
  adaptationBlocked: false,
  _listenerInstalled: false,

  ensureListener: () => {
    if (get()._listenerInstalled) return;
    set({ _listenerInstalled: true });
    // No initial fetch: "can this link keep up" is only knowable from
    // observed playback, so there's nothing meaningful to seed. The first
    // event lands within a watchdog poll of anything worth reporting.
    listen<PlaybackQualityPayload>("playback-quality", (event) => {
      set(event.payload);
    });
  },
}));
