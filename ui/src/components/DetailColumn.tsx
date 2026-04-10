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

  const scrollRef = useCallback(
    (el: HTMLDivElement | null) => {
      obsRef.current?.disconnect();
      elRef.current = el;
      if (el) {
        setPanelHeight(el.clientHeight);
        const obs = new ResizeObserver(() => {
          setPanelHeight(el.clientHeight);
          queue.setOpen(false);
        });
        obs.observe(el);
        obsRef.current = obs;
      }
    },
    [queue],
  );

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
