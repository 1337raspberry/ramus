import { useEffect } from "react";

import { useConnectionStore } from "../stores/connectionStore";
import type { ConnectionStatusPayload } from "./types";

/// React hook over the shared connection store. First mount installs the
/// `connection-status` event listener + fetches the initial state; every
/// subsequent subscriber just reads from the store.
export function useConnectionStatus(): ConnectionStatusPayload {
  const ensureListener = useConnectionStore((s) => s.ensureListener);
  const online = useConnectionStore((s) => s.online);
  const offlineModeManual = useConnectionStore((s) => s.offlineModeManual);
  const effectiveOffline = useConnectionStore((s) => s.effectiveOffline);

  useEffect(() => {
    ensureListener();
  }, [ensureListener]);

  return { online, offlineModeManual, effectiveOffline };
}
