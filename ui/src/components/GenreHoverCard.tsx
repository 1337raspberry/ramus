import { useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { peekGenreMetadata } from "../lib/genreMetadataCache";
import { useGenreHoverStore } from "../lib/genreSurface";

/**
 * The small dwell-hover summary card for desktop genre surfaces. Mounted once
 * at the app root; visibility and anchoring are driven entirely by the
 * hover controller in `lib/genreSurface.ts`. Pointer-events: none — it is
 * read-only, and right-clicking "through" it hits the anchor underneath.
 */
export default function GenreHoverCard() {
  const genre = useGenreHoverStore((s) => s.genre);
  const anchor = useGenreHoverStore((s) => s.anchor);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  // Measure after the hidden first paint, then place: below the anchor,
  // flipped above when it would spill past the viewport bottom, clamped
  // horizontally. No reposition-on-scroll — any scroll closes the card.
  useLayoutEffect(() => {
    setPos(null);
    if (!genre || !anchor || !anchor.isConnected || !ref.current) return;
    const margin = 8;
    const card = ref.current.getBoundingClientRect();
    const rect = anchor.getBoundingClientRect();
    let top = rect.bottom + margin;
    if (top + card.height > window.innerHeight - margin) {
      top = rect.top - margin - card.height;
    }
    const left = Math.min(
      Math.max(rect.left + rect.width / 2 - card.width / 2, margin),
      window.innerWidth - card.width - margin,
    );
    setPos({ top, left });
  }, [genre, anchor]);

  if (!genre) return null;
  const summary = peekGenreMetadata(genre)?.shortSummary;
  if (!summary) return null;

  return createPortal(
    <div
      ref={ref}
      className="genre-hover-card glass"
      role="tooltip"
      style={pos ? { top: pos.top, left: pos.left } : { top: 0, left: 0, visibility: "hidden" }}
    >
      <div className="genre-hover-hint">Right-click for details</div>
      <p className="genre-hover-summary">{summary}</p>
    </div>,
    document.body,
  );
}
