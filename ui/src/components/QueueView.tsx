import { useEffect, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { getArtUrl } from "../lib/commands";
import { formatDuration } from "../lib/format";
import { IconMusicNote, IconClose } from "./Icons";

function QueueTrackThumb({ thumb }: { thumb: string | null }) {
  const [src, setSrc] = useState<string | null>(null);
  const [err, setErr] = useState(false);

  useEffect(() => {
    if (!thumb) return;
    let cancelled = false;
    getArtUrl(thumb, 50)
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch(() => {
        if (!cancelled) setErr(true);
      });
    return () => {
      cancelled = true;
    };
  }, [thumb]);

  if (src && !err) {
    return <img className="queue-thumb" src={src} alt="" onError={() => setErr(true)} />;
  }
  return (
    <div className="queue-thumb queue-thumb-placeholder">
      <IconMusicNote />
    </div>
  );
}

export default function QueueView() {
  const queue = usePlaybackStore((s) => s.queue);
  const queueIndex = usePlaybackStore((s) => s.queueIndex);
  const removeQueueItem = usePlaybackStore((s) => s.removeQueueItem);
  const jumpToIndex = usePlaybackStore((s) => s.jumpToIndex);
  const [visibleCount, setVisibleCount] = useState(30);

  const upcomingStart = queueIndex + 1;
  const totalUpcoming = Math.max(queue.length - upcomingStart, 0);
  const visible = queue.slice(upcomingStart, upcomingStart + visibleCount);

  return (
    <div className="queue-view">
      <div className="queue-header">
        <span className="queue-title">Up Next</span>
        <span className="queue-count">{totalUpcoming} tracks</span>
      </div>
      {totalUpcoming === 0 ? (
        <div className="queue-empty">No upcoming tracks</div>
      ) : (
        <div className="queue-list">
          {visible.map((track, i) => {
            const globalIndex = upcomingStart + i;
            return (
              <div
                key={`${globalIndex}-${track.ratingKey}`}
                className="queue-row"
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
              </div>
            );
          })}
          {visibleCount < totalUpcoming && (
            <button className="queue-show-more" onClick={() => setVisibleCount((c) => c + 50)}>
              Show more ({totalUpcoming - visibleCount} remaining)
            </button>
          )}
        </div>
      )}
    </div>
  );
}
