import { usePlaybackStore } from "../stores/playbackStore";
import { togglePlayPause, nextTrack, previousTrack } from "../lib/commands";
import { lyricsEmptyMessage } from "../lib/lyricsStatus";
import { shownVisualizerMode } from "../lib/visualizerMode";
import { SINGALONG_QUERY } from "../lib/singalong";
import { useMediaQuery } from "../lib/useMediaQuery";
import WaveformSeekBar from "./WaveformSeekBar";
import MarqueeText from "./MarqueeText";
import VisualizerToggle from "./VisualizerToggle";
import {
  IconMusicNote,
  IconPrevious,
  IconPause,
  IconPlay,
  IconNext,
  IconExpandCorner,
  IconLyrics,
} from "./Icons";

/**
 * Switches the timed-lyrics strip (`FocusSingalong`) on and off. Lit
 * while on; dimmed while on with nothing to show for the track (no
 * lyrics, or lyrics without timestamps), with the reason in the tooltip.
 * Absent while the bars are drawn or the window is too narrow, since the
 * strip shows in neither.
 */
function SingalongToggle() {
  const on = usePlaybackStore((s) => s.clearLyrics);
  const toggle = usePlaybackStore((s) => s.toggleClearLyrics);
  const ridge = usePlaybackStore(
    (s) => shownVisualizerMode(s.visualizerMode, s.focusClear) === "ridge",
  );
  const lyrics = usePlaybackStore((s) => s.lyrics);
  const loading = usePlaybackStore((s) => s.lyricsLoading);
  const status = usePlaybackStore((s) => s.lyricsStatus);
  const wide = useMediaQuery(SINGALONG_QUERY);
  if (!ridge || !wide) return null;

  let state = "Lyrics on";
  let unavailable = false;
  if (loading || (!lyrics && status === null)) {
    state = "Loading lyrics";
  } else if (!lyrics) {
    state = lyricsEmptyMessage(status);
    unavailable = true;
  } else if (!lyrics.isSynced) {
    state = "No timed lyrics for this track";
    unavailable = true;
  }
  const title = on ? `${state} — click to hide lyrics` : "Show lyrics";

  return (
    <button
      className={`np-viz-btn focus-lyrics-btn${on ? " active" : ""}${on && unavailable ? " unavailable" : ""}`}
      onClick={toggle}
      title={title}
      aria-label={title}
      aria-pressed={on}
    >
      <IconLyrics />
    </button>
  );
}

interface Props {
  title: string;
  artist: string;
  /** Resolved art URL, or null while loading / when there is none. */
  artSrc: string | null;
  artErr: boolean;
  onArtError: () => void;
  /** Leave the clear screen and restore the full focus layout. */
  onExit: () => void;
}

/**
 * The corner player shown in focus mode's clear screen: a small cluster
 * at the top right, with no panel behind it, holding the art thumbnail,
 * title and artist, transport, the visualiser mode and lyrics buttons,
 * the seek bar and a button back to the full layout. Everything else in
 * focus mode is unmounted while it shows, so the visualiser has the
 * window to itself.
 */
export default function FocusClearPlayer({
  title,
  artist,
  artSrc,
  artErr,
  onArtError,
  onExit,
}: Props) {
  const status = usePlaybackStore((s) => s.status);

  return (
    <div className="focus-clear-player">
      <div className="focus-clear-top">
        {artSrc && !artErr ? (
          <img className="focus-clear-art" src={artSrc} alt="" onError={onArtError} />
        ) : (
          <div className="focus-clear-art focus-clear-art-placeholder">
            <IconMusicNote />
          </div>
        )}
        <div className="focus-clear-meta">
          <MarqueeText className="focus-clear-title">{title}</MarqueeText>
          <MarqueeText className="focus-clear-artist">{artist}</MarqueeText>
        </div>
        <div className="np-transport focus-clear-transport">
          <button className="np-transport-btn" onClick={() => previousTrack()} title="Previous">
            <IconPrevious />
          </button>
          <button
            className="np-transport-btn np-play-btn"
            onClick={() => togglePlayPause()}
            title={status === "playing" ? "Pause" : "Play"}
          >
            {status === "playing" ? <IconPause /> : <IconPlay />}
          </button>
          <button className="np-transport-btn" onClick={() => nextTrack()} title="Next">
            <IconNext />
          </button>
        </div>
        <VisualizerToggle />
        <SingalongToggle />
        <button className="focus-close-btn" onClick={onExit} title="Leave clear screen (Esc)">
          <IconExpandCorner size={16} />
        </button>
      </div>
      <WaveformSeekBar />
    </div>
  );
}
