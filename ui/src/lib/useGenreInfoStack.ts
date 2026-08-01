import { useCallback, useEffect, useState } from "react";

import { fetchGenreMetadata, peekGenreMetadata } from "./genreMetadataCache";
import type { GenreMetadata } from "./types";

export interface GenreInfoStack {
  /** Genre currently shown (last drill entry), or null when closed. */
  current: string | null;
  meta: GenreMetadata | null;
  loading: boolean;
  canGoBack: boolean;
  /** Push a genre onto the drill trail (wiki-style navigation). */
  drillInto: (genre: string) => void;
  goBack: () => void;
  /** Atomic pop-one-level-or-close, for back gestures. */
  popOrClose: (onClose: () => void) => void;
}

/**
 * Drill-trail + metadata state shared by the genre-info surfaces. The trail
 * resets whenever `target` changes, so openers must always pass through a
 * closed (null) state between opens — never retarget while already open.
 */
export function useGenreInfoStack(target: string | null): GenreInfoStack {
  // Drill-through trail of genre names; the last entry is what's shown.
  const [stack, setStack] = useState<string[]>([]);
  // Last settled fetch, keyed so a stale resolution can't leak across genres.
  // Covers the failure path too: errors resolve null but stay uncached, so
  // the shared cache alone can't tell "failed" from "still loading".
  const [resolved, setResolved] = useState<{ key: string; meta: GenreMetadata | null } | null>(
    null,
  );

  useEffect(() => {
    setStack(target ? [target] : []);
  }, [target]);

  const current = stack.length ? stack[stack.length - 1] : null;

  useEffect(() => {
    if (!current) return;
    const key = current.toLowerCase();
    if (peekGenreMetadata(current) !== undefined) return;
    let cancelled = false;
    void fetchGenreMetadata(current).then((meta) => {
      if (!cancelled) setResolved({ key, meta });
    });
    return () => {
      cancelled = true;
    };
  }, [current]);

  const goBack = useCallback(() => {
    setStack((s) => (s.length > 1 ? s.slice(0, -1) : s));
  }, []);

  const drillInto = useCallback((genre: string) => {
    setStack((s) => [...s, genre]);
  }, []);

  const popOrClose = useCallback((onClose: () => void) => {
    setStack((s) => {
      if (s.length > 1) return s.slice(0, -1);
      onClose();
      return s;
    });
  }, []);

  const key = current?.toLowerCase() ?? null;
  const cached = current ? peekGenreMetadata(current) : undefined;
  const settled = resolved != null && resolved.key === key;
  const meta = cached !== undefined ? cached : settled ? resolved.meta : null;
  const loading = current != null && cached === undefined && !settled;

  return {
    current,
    meta,
    loading,
    canGoBack: stack.length > 1,
    drillInto,
    goBack,
    popOrClose,
  };
}
