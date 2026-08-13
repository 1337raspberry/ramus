import { useCallback, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { usePlaybackStore } from "../stores/playbackStore";
import { ART_SIZE } from "../lib/commands";
import { useArtUrl } from "../lib/useArtUrl";
import { useListReorder } from "../lib/useListReorder";
import { formatDuration } from "../lib/format";
import { IconMusicNote, IconClose } from "./Icons";

/** Fixed queue row height (28px thumb + 2×4px padding) — the virtualizer and
 * the reorder drag both compute slots from it, so the CSS height must match. */
const QUEUE_ROW_HEIGHT = 36;

/** Rendered window ≈ viewport + 2× this, so fast wheel scrolling rarely
 * outruns the mounted rows while a 1000+ queue stops mattering. */
const OVERSCAN = 30;

/** Nearest scrollable ancestor: `.detail-scroll` in the side panel,
 * `.focus-controls-panel` in focus mode. Resolved structurally rather than
 * by class name so the queue keeps working if a host renames itself. */
function findScrollParent(el: HTMLElement): HTMLElement | null {
  let node: HTMLElement | null = el.parentElement;
  while (node) {
    const overflowY = getComputedStyle(node).overflowY;
    if (overflowY === "auto" || overflowY === "scroll") return node;
    node = node.parentElement;
  }
  return null;
}

function QueueTrackThumb({ thumb }: { thumb: string | null }) {
  const { artSrc: src, artErr: err, setArtErr: setErr } = useArtUrl(thumb, ART_SIZE.SMALL);

  if (src && !err) {
    return <img className="queue-thumb" src={src} alt="" onError={() => setErr(true)} />;
  }
  return (
    <div className="queue-thumb queue-thumb-placeholder">
      <IconMusicNote />
    </div>
  );
}

/**
 * Upcoming-queue list shared by the side panel and focus mode. Virtualized
 * against the host's scroller (the same shared-scroller recipe as the mobile
 * Up Next list: rows sit in flow between two padding spacers, because the
 * reorder drag writes translate transforms imperatively and would fight
 * virtualizer-owned positioning; `scrollMargin` is the rows block's offset
 * inside the scroller, measured as a rect difference).
 */
export default function QueueView() {
  const queue = usePlaybackStore((s) => s.queue);
  const queueIndex = usePlaybackStore((s) => s.queueIndex);
  const removeQueueItem = usePlaybackStore((s) => s.removeQueueItem);
  const moveQueueItem = usePlaybackStore((s) => s.moveQueueItem);
  const jumpToIndex = usePlaybackStore((s) => s.jumpToIndex);
  const clearQueue = usePlaybackStore((s) => s.clearQueue);

  const upcomingStart = queueIndex + 1;
  const upcoming = queue.slice(upcomingStart);

  const [scroller, setScroller] = useState<HTMLElement | null>(null);
  const rootRef = useCallback((el: HTMLDivElement | null) => {
    setScroller(el ? findScrollParent(el) : null);
  }, []);

  const rowsRef = useRef<HTMLDivElement>(null);
  const [listMargin, setListMargin] = useState(0);
  useEffect(() => {
    if (!scroller) return;
    const measure = () => {
      const rows = rowsRef.current;
      if (!rows) return;
      const next = Math.round(
        rows.getBoundingClientRect().top -
          scroller.getBoundingClientRect().top +
          scroller.scrollTop,
      );
      setListMargin((m) => (m === next ? m : next));
    };
    measure();
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [scroller]);

  const virtualizer = useVirtualizer({
    count: upcoming.length,
    getScrollElement: () => scroller,
    estimateSize: () => QUEUE_ROW_HEIGHT,
    overscan: OVERSCAN,
    scrollMargin: listMargin,
  });

  // The reorder hook works in upcoming-list space; the store takes absolute
  // queue indices. Its per-index refs only hold mounted rows — fine, a drag
  // can't reach an unmounted one.
  const { setRowRef, handleProps } = useListReorder({
    count: upcoming.length,
    rowHeight: QUEUE_ROW_HEIGHT,
    onReorder: (from, to) => moveQueueItem(upcomingStart + from, upcomingStart + to),
  });

  const items = virtualizer.getVirtualItems();
  const padTop = items.length > 0 ? items[0].start - listMargin : 0;
  const padBottom =
    items.length > 0 ? virtualizer.getTotalSize() + listMargin - items[items.length - 1].end : 0;

  return (
    <div className="queue-view" ref={rootRef}>
      <div className="queue-header">
        <span className="queue-title">Up Next</span>
        <span className="queue-count">{upcoming.length} tracks</span>
        <button
          className="queue-clear"
          onClick={clearQueue}
          disabled={queue.length === 0}
          title="Stop playback and empty the queue"
        >
          Clear
        </button>
      </div>
      {upcoming.length === 0 ? (
        <div className="queue-empty">No upcoming tracks</div>
      ) : (
        <div
          className="queue-list"
          ref={rowsRef}
          style={{ paddingTop: padTop, paddingBottom: padBottom }}
        >
          {items.map((vi) => {
            const i = vi.index;
            const track = upcoming[i];
            const globalIndex = upcomingStart + i;
            return (
              <div
                key={`${globalIndex}-${track.ratingKey}`}
                className="queue-row"
                ref={(el) => setRowRef(i, el)}
                onClick={() => jumpToIndex(globalIndex)}
              >
                <QueueTrackThumb thumb={track.thumb} />
                <div className="queue-track-info">
                  <div className="queue-track-title">{track.title}</div>
                  <div className="queue-track-artist">{track.trackArtist || track.artistName}</div>
                </div>
                <span className="queue-track-duration">{formatDuration(track.duration)}</span>
                <button
                  className="queue-remove"
                  onClick={(e) => {
                    e.stopPropagation();
                    removeQueueItem(globalIndex);
                  }}
                >
                  <IconClose size={12} />
                </button>
                <span
                  className="queue-grab"
                  aria-label="Reorder"
                  onClick={(e) => e.stopPropagation()}
                  {...handleProps(i)}
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
                    <circle cx="9" cy="6" r="1.8" />
                    <circle cx="15" cy="6" r="1.8" />
                    <circle cx="9" cy="12" r="1.8" />
                    <circle cx="15" cy="12" r="1.8" />
                    <circle cx="9" cy="18" r="1.8" />
                    <circle cx="15" cy="18" r="1.8" />
                  </svg>
                </span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
