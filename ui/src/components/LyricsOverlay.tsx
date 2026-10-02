import { usePlaybackStore } from "../stores/playbackStore";
import { lyricsEmptyMessage } from "../lib/lyricsStatus";
import LyricsView from "./LyricsView";
import { IconClose } from "./Icons";

/**
 * Shared lyrics overlay for Now Playing surfaces. Reads state directly
 * from `playbackStore`, so both NowPlayingView and FocusNowPlayingView
 * can drop it into their album-art container without prop drilling.
 * Renders nothing when `showLyrics` is false.
 *
 * The close button lives here rather than in LyricsView so the loading
 * and empty states keep an exit affordance — in focus mode the overlay
 * is a whole-panel takeover, and "No lyrics found" with no visible way
 * out strands the user. (The mobile sheet hides this button and renders
 * its own exit in the header row.)
 */
export default function LyricsOverlay() {
  const showLyrics = usePlaybackStore((s) => s.showLyrics);
  const lyrics = usePlaybackStore((s) => s.lyrics);
  const lyricsLoading = usePlaybackStore((s) => s.lyricsLoading);
  const lyricsStatus = usePlaybackStore((s) => s.lyricsStatus);
  const toggleLyrics = usePlaybackStore((s) => s.toggleLyrics);
  const seek = usePlaybackStore((s) => s.seek);

  if (!showLyrics) return null;

  return (
    <div className="np-lyrics-overlay">
      {/* stopPropagation: in the compact panel this overlay sits inside
          np-art-container, whose own click handler also toggles lyrics —
          letting the event bubble would re-open them immediately. */}
      <button
        className="lyrics-close"
        onClick={(e) => {
          e.stopPropagation();
          toggleLyrics();
        }}
        aria-label="Hide lyrics"
      >
        <IconClose size={14} />
      </button>
      {lyrics ? (
        <LyricsView lyrics={lyrics} onSeek={seek} />
      ) : lyricsLoading ? (
        <div className="lyrics-loading">loading lyrics...</div>
      ) : (
        <div className="lyrics-empty">{lyricsEmptyMessage(lyricsStatus)}</div>
      )}
    </div>
  );
}
