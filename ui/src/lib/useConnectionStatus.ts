import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import type { ConnectionStatusPayload } from "./types";
import { getConnectionStatus } from "./commands";

/// Subscribes to the `connection-status` event stream and syncs initial
/// state from `get_connection_status`. The payload combines live server
/// reachability with the user's manual Work Offline toggle into a single
/// `effectiveOffline` flag that consumers should use to decide whether
/// to render degraded / filtered UI.
export function useConnectionStatus(): ConnectionStatusPayload {
  const [status, setStatus] = useState<ConnectionStatusPayload>({
    online: true,
    offlineModeManual: false,
    effectiveOffline: false,
  });

  useEffect(() => {
    let cancelled = false;
    getConnectionStatus()
      .then((s) => {
        if (!cancelled) setStatus(s);
      })
      .catch(() => {});
    const unlisten = listen<ConnectionStatusPayload>("connection-status", (event) => {
      if (!cancelled) setStatus(event.payload);
    });
    return () => {
      cancelled = true;
      unlisten.then((fn) => fn());
    };
  }, []);

  return status;
}
