import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

const appWindow = getCurrentWindow();

/**
 * Track native fullscreen state and toggle a body class so styles can
 * hide the custom drag-region + reclaim the 32px it occupies at the top.
 * Tauri doesn't emit a dedicated fullscreen event, so we piggyback on
 * `onResized` (fires on enter/exit) and query `isFullscreen()`. macOS's
 * system menu bar still auto-reveals at the top edge, so the user can
 * exit via View > Exit Full Screen or ⌃⌘F when our chrome is hidden.
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
        // ignore
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
