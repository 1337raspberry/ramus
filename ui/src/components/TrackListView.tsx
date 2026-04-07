import { useLibraryStore } from "../stores/libraryStore";

function formatDuration(seconds: number): string {
  const mins = Math.floor(seconds / 60);
  const secs = Math.floor(seconds % 60);
  return `${mins}:${secs.toString().padStart(2, "0")}`;
}

function formatCodec(codec: string | null, bitrate: number | null): string | null {
  if (!codec) return null;
  const lossless = ["flac", "alac", "wav", "aiff", "pcm"];
  if (lossless.includes(codec.toLowerCase())) return codec.toUpperCase();
  if (bitrate) return `${codec.toUpperCase()} ${bitrate}`;
  return codec.toUpperCase();
}

export default function TrackListView() {
  const selectedAlbum = useLibraryStore((s) => s.selectedAlbum);
  const tracks = useLibraryStore((s) => s.tracks);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const toggleTrackFav = useLibraryStore((s) => s.toggleTrackFav);

  if (!selectedAlbum) {
    return <div className="empty-state">Select an album</div>;
  }

  return (
    <div>
      <div className="track-list-header">
        <h2>{selectedAlbum.title}</h2>
        <div className="album-meta">
          {selectedAlbum.artistName}
          {selectedAlbum.year ? ` · ${selectedAlbum.year}` : ""}
        </div>
      </div>
      <div className="track-list">
        {tracks.map((track, i) => {
          const badge = formatCodec(track.codec, track.bitrate);
          const hasTrackArtist =
            track.trackArtist &&
            track.trackArtist.toLowerCase() !== track.artistName.toLowerCase();

          return (
            <div
              key={track.ratingKey}
              className="track-row"
              onClick={() => playAlbum(selectedAlbum, i)}
            >
              <span className="track-number">{track.index ?? i + 1}</span>
              <div className="track-info">
                <div className="track-title">{track.title}</div>
                {hasTrackArtist && (
                  <div className="track-artist-inline">{track.trackArtist}</div>
                )}
              </div>
              {badge && <span className="track-format-badge">{badge}</span>}
              <button
                className={`track-fav-btn${track.isFavourite ? " visible" : ""}`}
                onClick={(e) => {
                  e.stopPropagation();
                  toggleTrackFav(track);
                }}
              >
                {track.isFavourite ? "★" : "☆"}
              </button>
              <span className="track-duration">
                {formatDuration(track.duration)}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
