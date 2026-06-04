import type { LyricsStatus } from "../lib/types";
import { usePlaybackStore } from "../stores/playbackStore";
import LyricsView from "./LyricsView";

/** Honest empty-state copy for a finished fetch that produced no lyrics. */
function emptyMessage(status: LyricsStatus | null): string {
  switch (status) {
    case "offline":
      return "Network unavailable";
    case "unreachable":
      return "Couldn't reach lyrics server";
    case "notFound":
      return "No lyrics found";
    default:
      return "No lyrics available";
  }
}

/**
 * Shared lyrics overlay for Now Playing surfaces. Reads state directly
 * from `playbackStore`, so both NowPlayingView and FocusNowPlayingView
 * can drop it into their album-art container without prop drilling.
 * Renders nothing when `showLyrics` is false.
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
      {lyrics ? (
        <LyricsView lyrics={lyrics} onSeek={seek} onDismiss={toggleLyrics} />
      ) : lyricsLoading ? (
        <div className="lyrics-loading">loading lyrics...</div>
      ) : (
        <div className="lyrics-empty">{emptyMessage(lyricsStatus)}</div>
      )}
    </div>
  );
}
