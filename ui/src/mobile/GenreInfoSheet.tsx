import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { IconChevronLeft } from "../components/Icons";
import { getGenreMetadata } from "../lib/commands";
import { pushBackHandler } from "../lib/backHandler";
import type { GenreMetadata } from "../lib/types";
import { useGenreInfoStore } from "../stores/genreInfoStore";
import { useLibraryStore } from "../stores/libraryStore";

interface Props {
  /** Run before navigating to a genre's albums — e.g. collapse the now-playing
   * sheet so the destination grid is visible. */
  onNavigate?: () => void;
}

/**
 * Reader popover for the richer genre metadata. Opened by long-pressing a genre
 * pill (via `genreInfoStore`); mounted once near the app root so it can float
 * above the now-playing sheet. Tapping a `**reference**` inside a description
 * drills into that genre (wiki-style, with a back affordance); tapping the
 * title navigates to that genre's albums.
 */
export default function GenreInfoSheet({ onNavigate }: Props) {
  const target = useGenreInfoStore((s) => s.target);
  const close = useGenreInfoStore((s) => s.close);
  const selectGenreByName = useLibraryStore((s) => s.selectGenreByName);
  const loadAlbumsForArtistName = useLibraryStore((s) => s.loadAlbumsForArtistName);

  // Drill-through trail of genre names; the last entry is what's shown.
  const [stack, setStack] = useState<string[]>([]);
  // Metadata keyed by lowercased name. A present `null` means "fetched, none".
  const [cache, setCache] = useState<Record<string, GenreMetadata | null>>({});
  const cacheRef = useRef(cache);
  cacheRef.current = cache;

  // Reset the trail whenever a pill opens the sheet fresh.
  useEffect(() => {
    setStack(target ? [target] : []);
  }, [target]);

  const current = stack.length ? stack[stack.length - 1] : null;

  // Lazily fetch metadata for the visible genre.
  useEffect(() => {
    if (!current) return;
    const key = current.toLowerCase();
    if (key in cacheRef.current) return;
    let cancelled = false;
    getGenreMetadata(current)
      .then((meta) => {
        if (!cancelled) setCache((c) => ({ ...c, [key]: meta }));
      })
      .catch(() => {
        if (!cancelled) setCache((c) => ({ ...c, [key]: null }));
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

  // Dismiss wiring while open: Android hardware back pops a drill level (or
  // closes at the root); Escape closes the whole sheet.
  useEffect(() => {
    if (!target) return;
    const popOrClose = () => {
      setStack((s) => {
        if (s.length > 1) return s.slice(0, -1);
        close();
        return s;
      });
    };
    const removeBack = pushBackHandler(() => {
      popOrClose();
      return true;
    });
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      removeBack();
      window.removeEventListener("keydown", onKey);
    };
  }, [target, close]);

  if (!target || !current) return null;

  const key = current.toLowerCase();
  const loading = !(key in cache);
  const meta = cache[key] ?? null;
  const shortSummary = meta?.shortSummary ?? null;
  const akas = meta?.cosmeticAka ?? [];
  const segments = meta?.descriptionSegments ?? [];
  const titleInLibrary = meta?.inLibrary ?? false;
  const showMinimal = !loading && !shortSummary && akas.length === 0 && segments.length === 0;

  const navigateToGenre = (genre: string) => {
    close();
    onNavigate?.();
    void selectGenreByName(genre);
  };

  const navigateToArtist = (artist: string) => {
    close();
    onNavigate?.();
    void loadAlbumsForArtistName(artist);
  };

  return createPortal(
    <div
      className="genre-info-backdrop"
      onClick={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div className="genre-info-sheet" role="dialog" aria-modal="true">
        <div className="genre-info-grabber" />
        <div className="genre-info-header">
          {stack.length > 1 && (
            <button className="genre-info-back" onClick={goBack} aria-label="Back">
              <IconChevronLeft size={22} />
            </button>
          )}
          {titleInLibrary ? (
            <button
              className="genre-info-title linked"
              onClick={() => navigateToGenre(current)}
              title="Show albums for this genre"
            >
              {meta?.canonicalName ?? current}
            </button>
          ) : (
            <span className="genre-info-title">{meta?.canonicalName ?? current}</span>
          )}
        </div>

        <div className="genre-info-body">
          {shortSummary && <p className="genre-info-short">{shortSummary}</p>}

          {akas.length > 0 && (
            <div className="genre-info-section">
              <div className="genre-info-label">AKA</div>
              <div className="genre-info-aka">
                {akas.map((aka, i) => (
                  <span key={aka}>
                    {i > 0 && <span className="genre-info-aka-sep"> · </span>}
                    {aka}
                  </span>
                ))}
              </div>
            </div>
          )}

          {segments.length > 0 && (
            <div className="genre-info-section">
              <p className="genre-info-summary">
                {segments.map((seg, i) => {
                  if (seg.kind === "text") return <span key={i}>{seg.value}</span>;
                  if (seg.kind === "genreLink") {
                    // Genre links always drill into the genre's info; the
                    // library flag only adds the bold + underlined treatment.
                    return (
                      <button
                        key={i}
                        className={`genre-link${seg.inLibrary ? " owned" : ""}`}
                        onClick={() => drillInto(seg.value)}
                      >
                        {seg.value}
                      </button>
                    );
                  }
                  // Artist links navigate only when owned; otherwise they're
                  // accent-coloured but non-interactive.
                  if (seg.inLibrary) {
                    const navName = seg.navName ?? seg.value;
                    return (
                      <button
                        key={i}
                        className="artist-link owned"
                        onClick={() => navigateToArtist(navName)}
                      >
                        {seg.value}
                      </button>
                    );
                  }
                  return (
                    <span key={i} className="artist-ref">
                      {seg.value}
                    </span>
                  );
                })}
              </p>
            </div>
          )}

          {showMinimal && <div className="genre-info-empty">No additional info</div>}
        </div>
      </div>
    </div>,
    document.body,
  );
}
