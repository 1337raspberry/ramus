import { useEffect, useState } from "react";

/**
 * Subscribes to a CSS media query and re-renders when it flips.
 * SSR-safe: false on the server, evaluated on mount.
 */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => {
    if (typeof window === "undefined") return false;
    return window.matchMedia(query).matches;
  });

  useEffect(() => {
    const mq = window.matchMedia(query);
    setMatches(mq.matches);
    const handler = (e: MediaQueryListEvent) => setMatches(e.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, [query]);

  return matches;
}

/**
 * Touch viewports with tablet-class width. The single-pane phone
 * navigation stays, but the surfaces get more room: a wider album grid,
 * larger now-playing art, a capped content column. Phones top out well
 * under this width in portrait (the widest is ~440px). Turned to
 * landscape (the full-screen visualiser does) a phone is wide enough, so
 * the height floor keeps it out: no phone is 500px tall in landscape.
 *
 * Keep in sync with the `(min-width: 700px)` media blocks in styles.css.
 * Nothing consults it from TypeScript yet: the tablet tier is CSS-only,
 * and this is the documented anchor for that breakpoint.
 */
export const TABLET_QUERY = "(pointer: coarse) and (min-width: 700px) and (min-height: 500px)";

/**
 * Wide enough to browse in two panes — navigation on the left, albums on
 * the right. Any iPad in landscape clears this; only the 13" iPad does in
 * portrait. A phone in landscape is held out by the same height floor as
 * `TABLET_QUERY`.
 *
 * Keep in sync with the `(min-width: 900px)` media blocks in styles.css.
 */
export const SPLIT_QUERY = "(pointer: coarse) and (min-width: 900px) and (min-height: 500px)";
