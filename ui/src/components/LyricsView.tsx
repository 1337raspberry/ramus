import { useEffect, useRef, useState } from "react";
import type { LyricsResult } from "../lib/types";
import { usePlaybackStore, activeLineIndex } from "../stores/playbackStore";
import { IconClose, IconPin } from "./Icons";

interface Props {
  lyrics: LyricsResult;
  isPinned: boolean;
  onTogglePin: () => void;
  onSeek: (seconds: number) => void;
  onDismiss: () => void;
}

export default function LyricsView({ lyrics, isPinned, onTogglePin, onSeek, onDismiss }: Props) {
  const position = usePlaybackStore((s) => s.position);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [flashId, setFlashId] = useState<number | null>(null);
  const lastActiveRef = useRef(-1);

  const active = activeLineIndex(lyrics, position);

  // Auto-scroll only when active line actually changes
  useEffect(() => {
    if (active < 0 || active === lastActiveRef.current) return;
    lastActiveRef.current = active;
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
        <IconClose size={14} />
      </button>
      <button className={`lyrics-pin${isPinned ? " pinned" : ""}`} onClick={onTogglePin}>
        <IconPin />
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
        <div className="lyrics-source">{lyrics.source === "plex" ? "via Plex" : "via LRCLIB"}</div>
      </div>
    </div>
  );
}
