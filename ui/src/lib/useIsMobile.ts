import { useEffect, useState } from "react";

// `pointer: coarse` matches devices whose primary input is touch — iOS,
// Android, handheld tablets. A desktop with an optional touchscreen stays
// `pointer: fine` because the mouse is the primary pointer. Crucially,
// resizing a desktop window doesn't change the pointer type, so shrinking
// the window no longer flips the UI into the stacked mobile layout.
const QUERY = "(pointer: coarse)";

/**
 * Returns true when the app is running on a touch-primary device
 * (iOS / Android / touch-first tablet). Used to branch the root App
 * between the desktop three-column layout and the stacked mobile layout.
 *
 * SSR-safe: returns false on the server and re-evaluates on mount.
 */
export function useIsMobile(): boolean {
  const [isMobile, setIsMobile] = useState(() => {
    if (typeof window === "undefined") return false;
    return window.matchMedia(QUERY).matches;
  });

  useEffect(() => {
    const mq = window.matchMedia(QUERY);
    const handler = (e: MediaQueryListEvent) => setIsMobile(e.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, []);

  return isMobile;
}
