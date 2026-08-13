import { useEffect, useRef, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { appendAlbumsToQueue } from "../lib/queueAllAlbums";
import { IconPlus } from "./Icons";

/**
 * Accent "+" in the grid header: appends every visible album's tracks to
 * the end of the now-playing queue, behind a small confirm popover (an
 * accidental click would otherwise dump an entire filtered library into
 * the queue). Hidden while the grid is empty.
 */
export default function QueueAllButton() {
  const albums = useLibraryStore((s) => s.albums);
  const [confirming, setConfirming] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!confirming) return;
    const handler = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setConfirming(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [confirming]);

  if (albums.length === 0) return null;

  return (
    <div className="filter-dropdown-wrap" ref={wrapRef}>
      <button
        className="filter-dropdown-btn queue-all-btn"
        onClick={() => setConfirming((v) => !v)}
        title="Add all albums to queue"
      >
        <IconPlus size={14} />
      </button>
      {confirming && (
        <div className="queue-all-confirm">
          <span>Add all albums to the now playing queue?</span>
          <div className="queue-all-actions">
            <button
              className="queue-all-yes"
              onClick={() => {
                setConfirming(false);
                void appendAlbumsToQueue(albums);
              }}
            >
              Add
            </button>
            <button className="queue-all-no" onClick={() => setConfirming(false)}>
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
