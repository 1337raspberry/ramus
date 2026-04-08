import { useCallback, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import NowPlayingView from "./NowPlayingView";

interface DetailColumnProps {
  onOpenEQ?: () => void;
}

export default function DetailColumn({ onOpenEQ }: DetailColumnProps) {
  const currentTrack = usePlaybackStore((s) => s.currentTrack);
  const [panelHeight, setPanelHeight] = useState(0);
  const obsRef = useRef<ResizeObserver | null>(null);

  const scrollRef = useCallback((el: HTMLDivElement | null) => {
    obsRef.current?.disconnect();
    if (el) {
      setPanelHeight(el.clientHeight);
      const obs = new ResizeObserver(() => setPanelHeight(el.clientHeight));
      obs.observe(el);
      obsRef.current = obs;
    }
  }, []);

  if (!currentTrack) {
    return <div className="empty-state">Select an album</div>;
  }

  return (
    <div ref={scrollRef} className="detail-scroll">
      <NowPlayingView onOpenEQ={onOpenEQ} panelHeight={panelHeight} />
    </div>
  );
}
