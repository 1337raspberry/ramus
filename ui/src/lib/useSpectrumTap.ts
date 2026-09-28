import { useEffect } from "react";
import { setSpectrumTap } from "./commands";

/**
 * Install and remove requests are chained so they reach the backend in
 * mount order, one at a time. A remount issues remove-then-install with no
 * gap (React runs an effect's cleanup and re-run back to back in
 * StrictMode, and a fast toggle does the same), and each command runs on
 * its own backend task, so unchained requests could complete in the
 * opposite order and leave the tap off while a visualiser is mounted.
 */
let tapRequests: Promise<void> = Promise.resolve();
/**
 * `performance.now()` when the document last became visible. A paint
 * loop's no-frames hint must not count a hidden stretch, when the tap was
 * deliberately removed.
 */
let visibleSince = 0;

function requestTap(enabled: boolean): void {
  tapRequests = tapRequests
    .then(() => setSpectrumTap(enabled))
    .catch((e) => console.warn(`[spectrum] tap ${enabled ? "install" : "remove"} failed:`, e));
}

// A fresh page owns no tap. A reload, or a restarted web content process,
// runs no unmount cleanup, so a tap the previous page installed would
// otherwise keep running (and its frames keep streaming) under a page with
// no visualiser mounted. Removing a tap that is not installed is a no-op.
requestTap(false);

/** When the document last became visible (`performance.now()`). */
export function tapVisibleSince(): number {
  return visibleSince;
}

/**
 * Keep the spectrum tap installed while `active` and the document is
 * visible, and removed otherwise: frames measured while nothing can paint
 * would cost the tap's CPU for nothing. Every request goes through the
 * chain, so a hide/show pair lands in order like any other toggle.
 */
export function useSpectrumTap(active: boolean): void {
  useEffect(() => {
    if (!active) return;
    const sync = () => {
      if (!document.hidden) visibleSince = performance.now();
      requestTap(!document.hidden);
    };
    sync();
    document.addEventListener("visibilitychange", sync);
    return () => {
      document.removeEventListener("visibilitychange", sync);
      requestTap(false);
    };
  }, [active]);
}
