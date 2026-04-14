import { usePlaybackStore } from "../stores/playbackStore";
import LyricsView from "./LyricsView";

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
  const lyricsPinned = usePlaybackStore((s) => s.lyricsPinned);
  const toggleLyrics = usePlaybackStore((s) => s.toggleLyrics);
  const toggleLyricsPinned = usePlaybackStore((s) => s.toggleLyricsPinned);
  const seek = usePlaybackStore((s) => s.seek);

  if (!showLyrics) return null;

  return (
    <div className="np-lyrics-overlay">
      {lyrics ? (
        <LyricsView
          lyrics={lyrics}
          isPinned={lyricsPinned}
          onTogglePin={toggleLyricsPinned}
          onSeek={seek}
          onDismiss={toggleLyrics}
        />
      ) : lyricsLoading ? (
        <div className="lyrics-loading">loading lyrics...</div>
      ) : (
        <div className="lyrics-empty">No lyrics available</div>
      )}
    </div>
  );
}
