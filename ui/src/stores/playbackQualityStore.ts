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
  /** The in-flight or settled `listen()` registration; null until first call. */
  _listenerInstalled: Promise<unknown> | null;
  /** Resolves once the subscription is actually live. The backend emits on
   *  change only, so an emit dropped for want of a subscriber is not resent —
   *  a steady degraded link would then go unreported for the whole session. */
  ensureListener: () => Promise<unknown>;
}

export const usePlaybackQualityStore = create<PlaybackQualityState>((set, get) => ({
  starving: false,
  degradedToKbps: null,
  adaptationBlocked: false,
  _listenerInstalled: null,

  ensureListener: () => {
    const existing = get()._listenerInstalled;
    if (existing) return existing;
    // No initial fetch: "can this link keep up" is only knowable from
    // observed playback, so there's nothing meaningful to seed. The first
    // event lands within a watchdog poll of anything worth reporting.
    const ready = listen<PlaybackQualityPayload>("playback-quality", (event) => {
      set(event.payload);
    });
    set({ _listenerInstalled: ready });
    return ready;
  },
}));
