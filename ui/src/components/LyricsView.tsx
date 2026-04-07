import { useEffect, useRef, useState } from "react";
import type { LyricsResult } from "../lib/types";
import { activeLineIndex } from "../stores/playbackStore";

interface Props {
  lyrics: LyricsResult;
  position: number;
  isPinned: boolean;
  onTogglePin: () => void;
  onSeek: (seconds: number) => void;
  onDismiss: () => void;
}

export default function LyricsView({
  lyrics,
  position,
  isPinned,
  onTogglePin,
  onSeek,
  onDismiss,
}: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [flashId, setFlashId] = useState<number | null>(null);

  const active = activeLineIndex(lyrics, position);

  // Auto-scroll to active line
  useEffect(() => {
    if (active < 0) return;
    const container = scrollRef.current;
    if (!container) return;
    const el = container.querySelector(`[data-line-index="${active}"]`);
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }, [active]);

  const handleLineTap = (lineIndex: number) => {
    const ts = lyrics.lines[lineIndex].timestamp;
    if (ts === null) return;
    setFlashId(lyrics.lines[lineIndex].id);
    onSeek(ts);
    setTimeout(() => setFlashId(null), 300);
  };

  return (
    <div className="lyrics-overlay" onClick={(e) => e.stopPropagation()}>
      <button className="lyrics-close" onClick={onDismiss}>
        x
      </button>
      <button
        className={`lyrics-pin${isPinned ? " pinned" : ""}`}
        onClick={onTogglePin}
      >
        {isPinned ? "\u{1F4CC}" : "\u{1F4CC}"}
      </button>
      <div className="lyrics-scroll" ref={scrollRef}>
        {lyrics.lines.map((line, i) => {
          const isActive = active === i;
          const isSynced = line.timestamp !== null;
          return (
            <div
              key={line.id}
              data-line-index={i}
              className={`lyrics-line${isActive ? " active" : ""}${isSynced ? " synced" : ""}${flashId === line.id ? " flash" : ""}`}
              onClick={() => handleLineTap(i)}
            >
              {line.text}
            </div>
          );
        })}
        <div className="lyrics-source">
          {lyrics.source === "plex" ? "via Plex" : "via LRCLIB"}
        </div>
      </div>
    </div>
  );
}
