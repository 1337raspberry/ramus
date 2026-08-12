import { memo, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Track } from "../lib/types";
import { ART_SIZE } from "../lib/commands";
import { useArtUrl } from "../lib/useArtUrl";
import { useListReorder } from "../lib/useListReorder";
import { useSwipeToDelete } from "../lib/useSwipeToDelete";
import { formatDuration } from "../lib/format";
import { IconMusicNote } from "../components/Icons";

/** Fixed Up Next row height (44px thumb + 2×6px padding) — the reorder drag
 * and the virtualizer both compute slots from it, so the CSS height must
 * match. */
export const UPNEXT_ROW_HEIGHT = 56;

/** Rendered window ≈ viewport (~15 rows) + 2× this — roughly 100 mounted
 * rows, a deliberate middle ground: big enough that a fast fling rarely
 * outruns the window, small enough that a 1000+ queue stops mattering. */
const OVERSCAN = 44;

function UpNextThumb({ thumb }: { thumb: string | null }) {
  const { artSrc: src, artErr: err, setArtErr: setErr } = useArtUrl(thumb, ART_SIZE.SMALL);

  if (src && !err) {
    return <img className="mobile-upnext-thumb" src={src} alt="" onError={() => setErr(true)} />;
  }
  return (
    <div className="mobile-upnext-thumb mobile-upnext-thumb-ph">
      <IconMusicNote size={14} />
    </div>
  );
}

interface Props {
  queue: Track[];
  queueIndex: number;
  /** The sheet body — the single scroller shared with now-playing page 1. */
  scrollBodyRef: React.RefObject<HTMLDivElement | null>;
  onJump: (globalIndex: number) => void;
  onRemove: (globalIndex: number) => void;
  onMove: (fromGlobal: number, toGlobal: number) => void;
}

/**
 * The Up Next queue list, virtualized and memoized.
 *
 * Isolated from MobileNowPlaying so the virtualizer's scroll-driven
 * re-renders (one per row boundary crossed) reconcile only this list, never
 * page 1 of the sheet. The store actions arriving as props are stable, so
 * the memo only breaks on real queue/index changes.
 *
 * The rows sit IN FLOW between two padding spacers rather than being
 * absolutely positioned — the reorder drag and swipe-to-remove hooks write
 * translate transforms on the rows imperatively, which would fight
 * virtualizer-owned positioning transforms. (Fixed row height, no
 * `measureElement` — same rules as `MobileAlbumGrid`.)
 *
 * `scrollMargin` is the rows block's offset inside the shared scroller
 * (page 1 is one viewport tall, plus the header). `virtualItem.start`
 * values include that margin and `getTotalSize()` doesn't — hence the
 * spacer arithmetic below.
 */
function UpNextListInner({ queue, queueIndex, scrollBodyRef, onJump, onRemove, onMove }: Props) {
  const upcomingStart = queueIndex + 1;
  const upcoming = queue.slice(upcomingStart);

  // Both gesture hooks work in upcoming-list space; the store takes
  // absolute queue indices. Their per-index refs only hold mounted rows —
  // fine, a drag can't reach an unmounted one.
  const { setRowRef, handleProps } = useListReorder({
    count: upcoming.length,
    rowHeight: UPNEXT_ROW_HEIGHT,
    onReorder: (from, to) => onMove(upcomingStart + from, upcomingStart + to),
  });
  // Swipe-left-to-remove replaces the old per-row ✕ button (which crowded
  // out the index column past 999 tracks).
  const swipe = useSwipeToDelete({ onDelete: (i) => onRemove(upcomingStart + i) });

  const rowsRef = useRef<HTMLDivElement>(null);
  const [listMargin, setListMargin] = useState(0);
  useEffect(() => {
    const measure = () => {
      const body = scrollBodyRef.current;
      const rows = rowsRef.current;
      if (!body || !rows) return;
      // Rect difference is translate-invariant, so this reads correctly
      // even while the sheet itself is mid-drag or resting collapsed.
      const next = Math.round(
        rows.getBoundingClientRect().top - body.getBoundingClientRect().top + body.scrollTop,
      );
      setListMargin((m) => (m === next ? m : next));
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [scrollBodyRef]);

  const virtualizer = useVirtualizer({
    count: upcoming.length,
    getScrollElement: () => scrollBodyRef.current,
    estimateSize: () => UPNEXT_ROW_HEIGHT,
    overscan: OVERSCAN,
    scrollMargin: listMargin,
  });

  const items = virtualizer.getVirtualItems();
  const padTop = items.length > 0 ? items[0].start - listMargin : 0;
  const padBottom =
    items.length > 0 ? virtualizer.getTotalSize() + listMargin - items[items.length - 1].end : 0;

  return (
    <div className="mobile-upnext">
      <div className="mobile-upnext-header">Up Next</div>
      <div ref={rowsRef} style={{ paddingTop: padTop, paddingBottom: padBottom }}>
        {items.map((vi) => {
          const i = vi.index;
          const t = upcoming[i];
          const globalIndex = upcomingStart + i;
          return (
            <div
              key={`${globalIndex}-${t.ratingKey}`}
              className="mobile-upnext-row swipe-row"
              ref={(el) => setRowRef(i, el)}
            >
              <div
                className="mobile-upnext-content swipe-row-content"
                role="button"
                tabIndex={0}
                ref={(el) => swipe.setContentRef(i, el)}
                {...swipe.contentProps(i)}
                onClick={() => onJump(globalIndex)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onJump(globalIndex);
                  }
                }}
              >
                <span className="mobile-upnext-num">{i + 1}</span>
                <UpNextThumb thumb={t.thumb} />
                <div className="mobile-upnext-info">
                  <div className="mobile-upnext-title">{t.title}</div>
                  <div className="mobile-upnext-artist">{t.trackArtist || t.artistName}</div>
                </div>
                <span className="mobile-upnext-duration">{formatDuration(t.duration)}</span>
                <span
                  className="mobile-upnext-grab"
                  aria-label="Reorder"
                  onClick={(e) => e.stopPropagation()}
                  {...handleProps(i)}
                >
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
                    <circle cx="9" cy="6" r="1.6" />
                    <circle cx="15" cy="6" r="1.6" />
                    <circle cx="9" cy="12" r="1.6" />
                    <circle cx="15" cy="12" r="1.6" />
                    <circle cx="9" cy="18" r="1.6" />
                    <circle cx="15" cy="18" r="1.6" />
                  </svg>
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** Memoized: scroll re-renders stay inside; parent renders only reach it
 * when the queue or play position actually change. */
const UpNextList = memo(UpNextListInner);
export default UpNextList;
