import { useEffect, useRef, useState, useCallback } from "react";
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

  const scrollRef = useCallback((el: HTMLDivElement | null) => {
    obsRef.current?.disconnect();
    elRef.current = el;
    if (el) {
      setPanelHeight(el.clientHeight);
      const obs = new ResizeObserver(() => {
        setPanelHeight(el.clientHeight);
        // Don't call setOpen(false) here — the queue collapses via
        // onScroll (scroll-to-top gesture) only. Closing on resize
        // would snap the queue shut whenever the window is resized.
      });
      obs.observe(el);
      obsRef.current = obs;
    }
  }, []);

  // Scroll down to reveal the track listing when the queue opens.
  // Kept in a useEffect so handleToggleQueue stays stable (no queue.open dep).
  useEffect(() => {
    if (queue.open && elRef.current) {
      requestAnimationFrame(() => {
        elRef.current?.scrollTo({ top: elRef.current.scrollHeight, behavior: "smooth" });
      });
    }
  }, [queue.open]);

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
        onToggleQueue={queue.toggle}
      />
    </div>
  );
}
