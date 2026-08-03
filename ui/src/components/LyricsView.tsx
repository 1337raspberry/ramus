import { useEffect, useRef, useState } from "react";
import type { LyricsResult } from "../lib/types";
import { usePlaybackStore, activeLineIndex } from "../stores/playbackStore";
import { IconClose } from "./Icons";

interface Props {
  lyrics: LyricsResult;
  onSeek: (seconds: number) => void;
  onDismiss: () => void;
}

export default function LyricsView({ lyrics, onSeek, onDismiss }: Props) {
  const position = usePlaybackStore((s) => s.position);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [flashId, setFlashId] = useState<number | null>(null);
  const lastActiveRef = useRef(-1);

  const active = activeLineIndex(lyrics, position);
  // Unsynced lyrics have no active line to highlight; flag the container
  // so styling can keep every line readable instead of dimming them all
  // as "upcoming".
  const anySynced = lyrics.lines.some((l) => l.timestamp !== null);

  // Auto-scroll only when the active line changes. Scroll the lyrics
  // container directly instead of scrollIntoView: scrollIntoView walks
  // every scrollable ancestor, and once this container clamps at its end
  // the leftover centering distance scrolls the ancestor instead
  // (overflow: hidden only blocks user gestures, not programmatic
  // scrolls). scrollTo self-clamps to [0, max], so the viewport stays
  // put and the highlight simply walks down the final screen of lines.
  useEffect(() => {
    if (active < 0 || active === lastActiveRef.current) return;
    lastActiveRef.current = active;
    const container = scrollRef.current;
    if (!container) return;
    const el = container.querySelector<HTMLElement>(`[data-line-index="${active}"]`);
    if (!el) return;
    const lineTop =
      el.getBoundingClientRect().top - container.getBoundingClientRect().top + container.scrollTop;
    container.scrollTo({
      top: lineTop - (container.clientHeight - el.offsetHeight) / 2,
      behavior: "smooth",
    });
  }, [active]);

  const handleLineTap = (lineIndex: number) => {
    const ts = lyrics.lines[lineIndex].timestamp;
    if (ts === null) return;
    setFlashId(lyrics.lines[lineIndex].id);
    onSeek(ts);
    setTimeout(() => setFlashId(null), 300);
  };

  return (
    <div
      className={`lyrics-overlay${anySynced ? "" : " unsynced"}`}
      onClick={(e) => e.stopPropagation()}
    >
      <button className="lyrics-close" onClick={onDismiss}>
        <IconClose size={14} />
      </button>
      <div className="lyrics-scroll" ref={scrollRef}>
        {lyrics.lines.map((line, i) => {
          const isActive = active === i;
          const isPast = active >= 0 && i < active;
          const isSynced = line.timestamp !== null;
          return (
            <div
              key={line.id}
              data-line-index={i}
              className={`lyrics-line${isActive ? " active" : ""}${isPast ? " past" : ""}${isSynced ? " synced" : ""}${flashId === line.id ? " flash" : ""}`}
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
