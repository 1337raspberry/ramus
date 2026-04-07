import { useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useLibraryStore } from "../stores/libraryStore";
import { getArtUrl, toggleAlbumFavourite, toggleTrackFavourite } from "../lib/commands";
import VolumeSlider from "./VolumeSlider";
import WaveformSeekBar from "./WaveformSeekBar";
import FlowLayout from "./FlowLayout";
import LyricsView from "./LyricsView";
import QueueView from "./QueueView";
import { togglePlayPause, nextTrack, previousTrack } from "../lib/commands";

function formatCodec(codec: string | null, bitrate: number | null): string | null {
  if (!codec) return null;
  const lossless = ["flac", "alac", "wav", "aiff", "pcm"];
  if (lossless.includes(codec.toLowerCase())) return codec.toUpperCase();
  if (bitrate) return `${codec.toUpperCase()} ${bitrate}`;
  return codec.toUpperCase();
}

interface NowPlayingProps {
  onOpenEQ?: () => void;
}

export default function NowPlayingView({ onOpenEQ }: NowPlayingProps) {
  const track = usePlaybackStore((s) => s.currentTrack);
  const status = usePlaybackStore((s) => s.status);
  const position = usePlaybackStore((s) => s.position);
  const duration = usePlaybackStore((s) => s.duration);
  const isBuffering = usePlaybackStore((s) => s.isBuffering);
  const bufferedFraction = usePlaybackStore((s) => s.bufferedFraction);
  const volume = usePlaybackStore((s) => s.volume);
  const changeVolume = usePlaybackStore((s) => s.changeVolume);
  const waveformLevels = usePlaybackStore((s) => s.waveformLevels);
  const lyrics = usePlaybackStore((s) => s.lyrics);
  const lyricsLoading = usePlaybackStore((s) => s.lyricsLoading);
  const showLyrics = usePlaybackStore((s) => s.showLyrics);
  const lyricsPinned = usePlaybackStore((s) => s.lyricsPinned);
  const toggleLyrics = usePlaybackStore((s) => s.toggleLyrics);
  const toggleLyricsPinned = usePlaybackStore((s) => s.toggleLyricsPinned);
  const seek = usePlaybackStore((s) => s.seek);
  const currentGenres = usePlaybackStore((s) => s.currentGenres);
  const showQueue = usePlaybackStore((s) => s.showQueue);
  const toggleQueue = usePlaybackStore((s) => s.toggleQueue);

  const selectedAlbum = useLibraryStore((s) => s.selectedAlbum);

  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artErr, setArtErr] = useState(false);
  const [artThumb, setArtThumb] = useState<string | null>(null);

  // Load art for current track
  const thumb = track?.thumb ?? selectedAlbum?.thumb;
  if (thumb && thumb !== artThumb) {
    setArtThumb(thumb);
    setArtErr(false);
    setArtSrc(null);
    getArtUrl(thumb, 600)
      .then(setArtSrc)
      .catch(() => setArtErr(true));
  }

  if (!track) return null;

  const albumTitle = track.albumTitle;
  const artistName = track.artistName;
  const hasTrackArtist =
    track.trackArtist &&
    track.trackArtist.toLowerCase() !== track.artistName.toLowerCase();
  const year = selectedAlbum?.year;
  const studio = selectedAlbum?.studio;
  const codec = formatCodec(track.codec, track.bitrate);
  const albumFav = selectedAlbum?.isFavourite ?? false;
  const trackFav = track.isFavourite;

  const handleAlbumFavToggle = () => {
    if (!selectedAlbum) return;
    const next = !albumFav;
    toggleAlbumFavourite(selectedAlbum.ratingKey, next).catch(() => {});
  };

  const handleTrackFavToggle = () => {
    const next = !trackFav;
    toggleTrackFavourite(track.ratingKey, next).catch(() => {});
  };

  const handleGenreClick = (genre: string) => {
    const store = useLibraryStore.getState();
    store.setSidebarMode("genres");
    store.loadAlbumsForGenre(genre);
  };

  return (
    <div className="now-playing">
      {/* Header: Artist, Album, Year */}
      <div className="np-header">
        <div className="np-artist">
          {hasTrackArtist
            ? `${artistName} (${track.trackArtist})`
            : artistName}
        </div>
        <div className="np-album-row">
          <span className="np-album-title">{albumTitle}</span>
          <button
            className={`np-fav-btn${albumFav ? " active" : ""}`}
            onClick={handleAlbumFavToggle}
          >
            {albumFav ? "\u2605" : "\u2606"}
          </button>
        </div>
        {year && <div className="np-year">{year}</div>}
      </div>

      {/* Album art with lyrics overlay */}
      <div className="np-art-container" onClick={toggleLyrics}>
        {artSrc && !artErr ? (
          <img
            className="np-art"
            src={artSrc}
            alt={albumTitle}
            onError={() => setArtErr(true)}
          />
        ) : (
          <div className="np-art-placeholder">{"\u266B"}</div>
        )}
        {showLyrics && (
          <div className="np-lyrics-overlay">
            {lyrics ? (
              <LyricsView
                lyrics={lyrics}
                position={position}
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
        )}
      </div>

      {/* Volume */}
      <VolumeSlider value={volume} onChange={changeVolume} />

      {/* Track title + EQ + fav */}
      <div className="np-track-row">
        <span className="np-track-title">{track.title}</span>
        {onOpenEQ && (
          <button className="np-eq-btn" onClick={onOpenEQ} title="Equalizer">
            {"\u2261"}
          </button>
        )}
        <button
          className={`np-fav-btn${trackFav ? " active" : ""}`}
          onClick={(e) => {
            e.stopPropagation();
            handleTrackFavToggle();
          }}
        >
          {trackFav ? "\u2605" : "\u2606"}
        </button>
      </div>

      {/* Waveform seek bar */}
      {duration > 0 && (
        <WaveformSeekBar
          levels={waveformLevels}
          position={position}
          duration={duration}
          bufferedFraction={bufferedFraction}
          isBuffering={isBuffering}
          onSeek={seek}
        />
      )}

      {/* Transport controls */}
      <div className="np-transport">
        <button className="np-transport-btn" onClick={() => previousTrack()}>
          {"\u23EE"}
        </button>
        <button
          className="np-transport-btn np-play-btn"
          onClick={() => togglePlayPause()}
        >
          {status === "playing" ? "\u23F8" : "\u25B6"}
        </button>
        <button className="np-transport-btn" onClick={() => nextTrack()}>
          {"\u23ED"}
        </button>
      </div>

      {/* Footer: genres, studio, format */}
      <div className="np-footer">
        <FlowLayout genres={currentGenres} onGenreClick={handleGenreClick} />
        {(studio || codec) && (
          <div className="np-meta-row">
            {studio && <span className="np-studio">{studio}</span>}
            {codec && <span className="np-format">{codec}</span>}
          </div>
        )}
      </div>

      {/* Queue toggle */}
      <button className="np-queue-toggle" onClick={toggleQueue}>
        {showQueue ? "\u25B2" : "\u25BC"} Queue
      </button>

      {showQueue && <QueueView />}
    </div>
  );
}
