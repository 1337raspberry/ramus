import { useEffect, useLayoutEffect, useMemo, useRef } from "react";
import type { LyricLine, LyricsResult } from "../lib/types";
import { shownVisualizerMode } from "../lib/visualizerMode";
import { SINGALONG_PARAMS, SINGALONG_QUERY, placeSingalongLines } from "../lib/singalong";
import { useMediaQuery } from "../lib/useMediaQuery";
import { usePlaybackStore, activeLineIndex } from "../stores/playbackStore";

/**
 * Holds the focus slot before the first timed line is reached, so that
 * line waits in the "next" slot and rolls up into focus like any other.
 * A no-break space gives it one row's height.
 */
const LEAD_IN: LyricLine = { id: -1, timestamp: null, text: " " };

/**
 * Timed lyrics for the clear screen: the current line in focus in the
 * strip above the ridge, with the lines before and after it smaller and
 * faded. Shown while the corner player's lyrics toggle is on, the ridge
 * is the mode drawn (the bars hang through the same strip), the window
 * is wide enough to fit it beside the corner player, and the track's
 * lyrics carry timestamps; untimed lyrics have no current line to
 * follow, so they show nothing here.
 */
export default function FocusSingalong() {
  const on = usePlaybackStore((s) => s.clearLyrics);
  const ridge = usePlaybackStore(
    (s) => shownVisualizerMode(s.visualizerMode, s.focusClear) === "ridge",
  );
  const lyrics = usePlaybackStore((s) => s.lyrics);
  const wide = useMediaQuery(SINGALONG_QUERY);
  if (!on || !ridge || !wide || !lyrics?.isSynced) return null;
  return <SingalongLines lyrics={lyrics} />;
}

function SingalongLines({ lyrics }: { lyrics: LyricsResult }) {
  const { timed, rows } = useMemo(() => {
    const lines = lyrics.lines.filter((l) => l.timestamp !== null);
    return { timed: { ...lyrics, lines }, rows: [LEAD_IN, ...lines] };
  }, [lyrics]);
  // Selecting the index rather than the position re-renders on a line
  // change only, not on every position tick. `+ 1` steps over the lead-in
  // row, so "before the first line" (-1) lands on it.
  const active = usePlaybackStore(
    (s) => activeLineIndex(timed, s.position + SINGALONG_PARAMS.leadS) + 1,
  );
  const stripRef = useRef<HTMLDivElement>(null);

  // Two rows either side of the focus: the outer ones are invisible, and
  // give the line leaving at the top somewhere to fade out to and the
  // line arriving at the bottom somewhere to fade in from.
  const first = Math.max(0, active - 2);
  const shown = rows.slice(first, active + 3).map((line, i) => ({
    line,
    slot: first + i - active,
  }));

  useLayoutEffect(() => {
    if (stripRef.current) placeSingalongLines(stripRef.current);
  }, [active, rows]);

  // The type scales with the window, so a resize changes line heights
  // and wrap points.
  useEffect(() => {
    const strip = stripRef.current;
    if (!strip) return;
    const observer = new ResizeObserver(() => placeSingalongLines(strip));
    observer.observe(strip);
    return () => observer.disconnect();
  }, []);

  return (
    <div className="focus-singalong" ref={stripRef}>
      {shown.map(({ line, slot }) => (
        <div key={line.id} className="focus-singalong-line" data-slot={slot}>
          {line.text}
        </div>
      ))}
    </div>
  );
}
