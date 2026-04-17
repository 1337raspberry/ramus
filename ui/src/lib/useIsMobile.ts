import { useEffect, useState } from "react";

const MOBILE_BREAKPOINT_PX = 820;
const QUERY = `(max-width: ${MOBILE_BREAKPOINT_PX}px)`;

/**
 * Returns true when the viewport is phone/narrow-tablet sized. Used to
 * branch the root App between the desktop three-column layout and the
 * stacked mobile layout.
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
