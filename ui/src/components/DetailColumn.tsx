import { useCallback, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import NowPlayingView from "./NowPlayingView";

interface DetailColumnProps {
  onOpenEQ?: () => void;
}

export default function DetailColumn({ onOpenEQ }: DetailColumnProps) {
  const currentTrack = usePlaybackStore((s) => s.currentTrack);
  const [panelHeight, setPanelHeight] = useState(0);
  const [showQueue, setShowQueue] = useState(false);
  const showQueueRef = useRef(false);
  const elRef = useRef<HTMLDivElement | null>(null);
  const obsRef = useRef<ResizeObserver | null>(null);

  const setQueue = useCallback((open: boolean) => {
    showQueueRef.current = open;
    setShowQueue(open);
  }, []);

  const scrollRef = useCallback(
    (el: HTMLDivElement | null) => {
      obsRef.current?.disconnect();
      elRef.current = el;
      if (el) {
        setPanelHeight(el.clientHeight);
        const obs = new ResizeObserver(() => {
          setPanelHeight(el.clientHeight);
          setQueue(false);
        });
        obs.observe(el);
        obsRef.current = obs;
      }
    },
    [setQueue],
  );

  // Wheel down on the panel opens the queue
  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      if (e.deltaY > 0 && !showQueueRef.current) {
        setQueue(true);
      }
    },
    [setQueue],
  );

  // When scrolled back to top, close the queue
  const handleScroll = useCallback(
    (e: React.UIEvent<HTMLDivElement>) => {
      if (showQueueRef.current && e.currentTarget.scrollTop === 0) {
        setQueue(false);
      }
    },
    [setQueue],
  );

  if (!currentTrack) {
    return <div className="empty-state">Select an album</div>;
  }

  return (
    <div
      ref={scrollRef}
      className={`detail-scroll${showQueue ? " queue-open" : ""}`}
      onWheel={handleWheel}
      onScroll={handleScroll}
    >
      <NowPlayingView
        onOpenEQ={onOpenEQ}
        panelHeight={panelHeight}
        showQueue={showQueue}
        onToggleQueue={() => setQueue(!showQueueRef.current)}
      />
    </div>
  );
}
