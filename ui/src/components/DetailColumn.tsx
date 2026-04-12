import { useCallback, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useQueuePanel } from "../lib/useQueuePanel";
import NowPlayingView from "./NowPlayingView";

interface DetailColumnProps {
  onOpenEQ?: () => void;
}

export default function DetailColumn({ onOpenEQ }: DetailColumnProps) {
  const currentTrack = usePlaybackStore((s) => s.currentTrack);
  const [panelHeight, setPanelHeight] = useState(0);
  const queue = useQueuePanel();
  const elRef = useRef<HTMLDivElement | null>(null);
  const obsRef = useRef<ResizeObserver | null>(null);

  // queue.setOpen is a stable useCallback — depend on it rather than
  // the whole queue object, whose identity changes every time `open`
  // toggles.  Depending on `queue` caused the ResizeObserver to be
  // recreated on toggle, and its initial fire immediately called
  // setOpen(false), snapping the queue shut before it could render.
  const queueSetOpen = queue.setOpen;

  const scrollRef = useCallback(
    (el: HTMLDivElement | null) => {
      obsRef.current?.disconnect();
      elRef.current = el;
      if (el) {
        setPanelHeight(el.clientHeight);
        const obs = new ResizeObserver(() => {
          setPanelHeight(el.clientHeight);
          queueSetOpen(false);
        });
        obs.observe(el);
        obsRef.current = obs;
      }
    },
    [queueSetOpen],
  );

  const handleToggleQueue = useCallback(() => {
    queue.toggle();
    // Scroll down to reveal the track listing when opening
    if (!queue.open && elRef.current) {
      requestAnimationFrame(() => {
        elRef.current?.scrollTo({ top: elRef.current.scrollHeight, behavior: "smooth" });
      });
    }
  }, [queue.open, queue.toggle]);

  if (!currentTrack) {
    return <div className="empty-state">Select an album</div>;
  }

  return (
    <div
      ref={scrollRef}
      className={`detail-scroll${queue.open ? " queue-open" : ""}`}
      onWheel={queue.onWheel}
      onScroll={queue.onScroll}
    >
      <NowPlayingView
        onOpenEQ={onOpenEQ}
        panelHeight={panelHeight}
        showQueue={queue.open}
        onToggleQueue={handleToggleQueue}
      />
    </div>
  );
}
