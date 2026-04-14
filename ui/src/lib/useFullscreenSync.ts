import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

const appWindow = getCurrentWindow();

/**
 * Mirror native fullscreen state into a body class so styles can hide the
 * custom drag region and reclaim the 32px it occupies.
 *
 * Tauri has no dedicated fullscreen event; this piggybacks on `onResized`
 * (fires on enter/exit) and queries `isFullscreen()`.
 */
export function useFullscreenSync(): void {
  useEffect(() => {
    let cancelled = false;
    let unlistenResize: (() => void) | null = null;

    const apply = (fs: boolean) => {
      document.body.classList.toggle("is-fullscreen", fs);
    };

    const check = async () => {
      try {
        const fs = await appWindow.isFullscreen();
        if (!cancelled) apply(fs);
      } catch {
        /* ignore */
      }
    };

    check();

    appWindow
      .onResized(() => check())
      .then((fn) => {
        if (cancelled) fn();
        else unlistenResize = fn;
      });

    return () => {
      cancelled = true;
      unlistenResize?.();
      document.body.classList.remove("is-fullscreen");
    };
  }, []);
}
