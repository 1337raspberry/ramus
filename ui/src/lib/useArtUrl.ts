import { useEffect, useRef, useState } from "react";
import { create } from "zustand";
import { getArtUrl } from "./commands";

/// Global retry signal for failed art loads. Bumped when the backend
/// reports freshly warmed art (`metadata-warmed`) or the connection comes
/// back online — both moments where a previously failed fetch is likely to
/// succeed from the on-disk cache without touching the network. Hooks that
/// already resolved a URL ignore the bump, so only placeholder slots
/// re-request.
const useArtRetryStore = create<{ epoch: number }>(() => ({ epoch: 0 }));

export function bumpArtRetry(): void {
  useArtRetryStore.setState((s) => ({ epoch: s.epoch + 1 }));
}

/**
 * Load an album art URL at the given size tier, returning the resolved
 * cache path + an error flag. Cancels on unmount and on thumb/size change
 * so a late-arriving resolution from a previous track can never land on
 * the new one.
 *
 * Unresolved (failed or still-pending) loads retry when the art retry
 * epoch bumps; a resolved URL is kept as-is, so bumps never cause a
 * loaded image to flicker or refetch.
 *
 * Returns `{ artSrc: null, artErr: false }` when `thumb` is null — callers
 * render a placeholder in that case.
 */
export function useArtUrl(thumb: string | null | undefined, size: number) {
  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artErr, setArtErr] = useState(false);
  const retryEpoch = useArtRetryStore((s) => s.epoch);
  // `${size}::${thumb}` key the current artSrc resolved for — the guard
  // that makes epoch bumps a no-op for already-loaded slots.
  const loadedKey = useRef<string | null>(null);

  useEffect(() => {
    if (!thumb) {
      loadedKey.current = null;
      setArtSrc(null);
      setArtErr(false);
      return;
    }
    const key = `${size}::${thumb}`;
    if (loadedKey.current === key) return;
    loadedKey.current = null;
    setArtErr(false);
    setArtSrc(null);
    let cancelled = false;
    getArtUrl(thumb, size)
      .then((url) => {
        if (!cancelled) {
          loadedKey.current = key;
          setArtSrc(url);
        }
      })
      .catch(() => {
        if (!cancelled) setArtErr(true);
      });
    return () => {
      cancelled = true;
    };
  }, [thumb, size, retryEpoch]);

  return { artSrc, artErr, setArtErr };
}
