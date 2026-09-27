import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";

import { getCastStatus } from "../lib/commands";
import { refreshQueue } from "../lib/refreshQueue";
import type { CastPlayerRef, CastStatusPayload } from "../lib/types";
import { useToastStore } from "../components/Toast";
import { usePlaybackStore } from "./playbackStore";

/// The active cast, replayed from `cast-status`. Readable from non-React
/// code via `useCastStore.getState()`, like `connectionStore`.
interface CastState {
  player: CastPlayerRef | null;
  link: "connected" | "lost" | null;
  queueRevision: number;
  /** The in-flight or settled `listen()` registration; null until first call. */
  _listenerInstalled: Promise<unknown> | null;
  /** Resolves once the subscription is live; see `connectionStore`. */
  ensureListener: () => Promise<unknown>;
}

export const useCastStore = create<CastState>((set, get) => {
  const apply = (s: CastStatusPayload) => {
    const prev = get();
    // The queue shown moved (a cast's queue changed, or a cast began or
    // ended): Up Next only refreshes on track changes otherwise.
    if (s.queueRevision !== prev.queueRevision) void refreshQueue();
    if (s.notice) useToastStore.getState().show(s.notice);
    // The visualiser analyses local audio, which stops while casting.
    if (s.player && !prev.player) usePlaybackStore.getState().setMobileVisualizerOpen(false);
    set({ player: s.player, link: s.link, queueRevision: s.queueRevision });
  };

  return {
    player: null,
    link: null,
    queueRevision: 0,
    _listenerInstalled: null,

    ensureListener: () => {
      const existing = get()._listenerInstalled;
      if (existing) return existing;
      const ready = listen<CastStatusPayload>("cast-status", (event) => apply(event.payload));
      set({ _listenerInstalled: ready });
      getCastStatus()
        .then(apply)
        .catch(() => {});
      return ready;
    },
  };
});
