import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";

import { getConnectionStatus } from "../lib/commands";
import type { ConnectionStatusPayload } from "../lib/types";

/// Shared connection state. Subscribed to the `connection-status` event
/// stream so any module — React component, zustand store, or plain
/// async helper — can read the latest state synchronously via
/// `useConnectionStore.getState()`.
interface ConnectionState extends ConnectionStatusPayload {
  /** The in-flight or settled `listen()` registration; null until first call. */
  _listenerInstalled: Promise<unknown> | null;
  /** Resolves once the subscription is actually live — callers that trigger a
   *  re-emit must await it, since Tauri drops events with no subscriber. */
  ensureListener: () => Promise<unknown>;
}

export const useConnectionStore = create<ConnectionState>((set, get) => ({
  online: true,
  offlineModeManual: false,
  effectiveOffline: false,
  _listenerInstalled: null,

  ensureListener: () => {
    const existing = get()._listenerInstalled;
    if (existing) return existing;
    const ready = listen<ConnectionStatusPayload>("connection-status", (event) => {
      set(event.payload);
    });
    set({ _listenerInstalled: ready });
    getConnectionStatus()
      .then((s) => set(s))
      .catch(() => {});
    return ready;
  },
}));
