import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";

import { getConnectionStatus } from "../lib/commands";
import type { ConnectionStatusPayload } from "../lib/types";

/// Shared connection state. Subscribed to the `connection-status` event
/// stream so any module — React component, zustand store, or plain
/// async helper — can read the latest state synchronously via
/// `useConnectionStore.getState()`.
interface ConnectionState extends ConnectionStatusPayload {
  _listenerInstalled: boolean;
  ensureListener: () => void;
}

export const useConnectionStore = create<ConnectionState>((set, get) => ({
  online: true,
  offlineModeManual: false,
  effectiveOffline: false,
  _listenerInstalled: false,

  ensureListener: () => {
    if (get()._listenerInstalled) return;
    set({ _listenerInstalled: true });
    getConnectionStatus()
      .then((s) => set(s))
      .catch(() => {});
    listen<ConnectionStatusPayload>("connection-status", (event) => {
      set(event.payload);
    });
  },
}));
