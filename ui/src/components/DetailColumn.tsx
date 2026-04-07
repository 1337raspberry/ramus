import { useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useLibraryStore } from "../stores/libraryStore";
import NowPlayingView from "./NowPlayingView";
import TrackListView from "./TrackListView";

type DetailMode = "auto" | "tracks" | "nowPlaying";

export default function DetailColumn() {
  const currentTrack = usePlaybackStore((s) => s.currentTrack);
  const selectedAlbum = useLibraryStore((s) => s.selectedAlbum);
  const [mode, setMode] = useState<DetailMode>("auto");

  const isPlaying = currentTrack !== null;
  const hasAlbum = selectedAlbum !== null;

  // Determine what to show
  const showNowPlaying =
    mode === "nowPlaying" ||
    (mode === "auto" && isPlaying);

  // If nothing to show at all
  if (!isPlaying && !hasAlbum) {
    return <div className="empty-state">Select an album</div>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {/* Toggle tabs when both views are available */}
      {isPlaying && hasAlbum && (
        <div className="detail-tabs">
          <button
            className={`detail-tab${showNowPlaying ? " active" : ""}`}
            onClick={() => setMode(showNowPlaying ? "tracks" : "nowPlaying")}
          >
            {showNowPlaying ? "Tracks" : "Now Playing"}
          </button>
        </div>
      )}
      <div style={{ flex: 1, overflow: "auto" }}>
        {showNowPlaying && isPlaying ? <NowPlayingView /> : <TrackListView />}
      </div>
    </div>
  );
}
