import { useEffect, useState } from "react";
import { getArtUrl } from "./commands";

/**
 * Load an album art URL at the given size tier, returning the resolved
 * cache path + an error flag. Cancels on unmount and on thumb/size change
 * so a late-arriving resolution from a previous track can never land on
 * the new one.
 *
 * Returns `{ artSrc: null, artErr: false }` when `thumb` is null — callers
 * render a placeholder in that case.
 */
export function useArtUrl(thumb: string | null | undefined, size: number) {
  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artErr, setArtErr] = useState(false);

  useEffect(() => {
    if (!thumb) {
      setArtSrc(null);
      setArtErr(false);
      return;
    }
    setArtErr(false);
    setArtSrc(null);
    let cancelled = false;
    getArtUrl(thumb, size)
      .then((url) => {
        if (!cancelled) setArtSrc(url);
      })
      .catch(() => {
        if (!cancelled) setArtErr(true);
      });
    return () => {
      cancelled = true;
    };
  }, [thumb, size]);

  return { artSrc, artErr, setArtErr };
}
