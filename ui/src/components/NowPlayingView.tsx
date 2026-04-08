import { useCallback, useEffect, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useLibraryStore } from "../stores/libraryStore";
import { getArtUrl, toggleAlbumFavourite, toggleTrackFavourite } from "../lib/commands";
import { extractVibrantColor } from "../lib/vibrantColor";
import WaveformSeekBar from "./WaveformSeekBar";
import VolumeSlider from "./VolumeSlider";
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
  panelHeight?: number;
}

export default function NowPlayingView({ onOpenEQ, panelHeight }: NowPlayingProps) {
  const track = usePlaybackStore((s) => s.currentTrack);
  const status = usePlaybackStore((s) => s.status);
  const lyrics = usePlaybackStore((s) => s.lyrics);
  const lyricsLoading = usePlaybackStore((s) => s.lyricsLoading);
  const showLyrics = usePlaybackStore((s) => s.showLyrics);
  const lyricsPinned = usePlaybackStore((s) => s.lyricsPinned);
  const toggleLyrics = usePlaybackStore((s) => s.toggleLyrics);
  const toggleLyricsPinned = usePlaybackStore((s) => s.toggleLyricsPinned);
  const seek = usePlaybackStore((s) => s.seek);
  const currentGenres = usePlaybackStore((s) => s.currentGenres);
  const volume = usePlaybackStore((s) => s.volume);
  const changeVolume = usePlaybackStore((s) => s.changeVolume);

  const nowPlayingAlbum = usePlaybackStore((s) => s.nowPlayingAlbum);

  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artErr, setArtErr] = useState(false);
  const lastAccentThumb = useRef<string | null>(null);

  const thumb = track?.thumb ?? nowPlayingAlbum?.thumb ?? null;
  useEffect(() => {
    if (!thumb) return;
    setArtErr(false);
    setArtSrc(null);
    let cancelled = false;
    getArtUrl(thumb, 600)
      .then((url) => { if (!cancelled) setArtSrc(url); })
      .catch(() => { if (!cancelled) setArtErr(true); });
    return () => { cancelled = true; };
  }, [thumb]);

  const handleArtLoad = useCallback((e: React.SyntheticEvent<HTMLImageElement>) => {
    const img = e.currentTarget;
    if (lastAccentThumb.current === thumb) return;
    lastAccentThumb.current = thumb;
    const color = extractVibrantColor(img);
    if (color) {
      document.documentElement.style.setProperty("--accent-r", String(color[0]));
      document.documentElement.style.setProperty("--accent-g", String(color[1]));
      document.documentElement.style.setProperty("--accent-b", String(color[2]));
    }
  }, [thumb]);

  if (!track) return null;

  const albumTitle = track.albumTitle;
  const artistName = track.artistName;
  const hasTrackArtist =
    track.trackArtist &&
    track.trackArtist.toLowerCase() !== track.artistName.toLowerCase();
  const year = nowPlayingAlbum?.year;
  const studio = nowPlayingAlbum?.studio;
  const codec = formatCodec(track.codec, track.bitrate);
  const albumFav = nowPlayingAlbum?.isFavourite ?? false;
  const trackFav = track.isFavourite;

  const handleAlbumFavToggle = () => {
    if (!nowPlayingAlbum) return;
    toggleAlbumFavourite(nowPlayingAlbum.ratingKey, !albumFav).catch(() => {});
  };

  const handleTrackFavToggle = () => {
    toggleTrackFavourite(track.ratingKey, !trackFav).catch(() => {});
  };

  const handleArtistClick = () => {
    useLibraryStore.getState().loadAlbumsForArtistName(track.artistName);
  };

  const handleAlbumClick = () => {
    if (!nowPlayingAlbum) return;
    useLibraryStore.getState().openAlbumDetail(nowPlayingAlbum);
  };

  const handleYearClick = () => {
    if (year) useLibraryStore.getState().loadAlbumsForYear(year);
  };

  const handleGenreClick = (genre: string) => {
    const store = useLibraryStore.getState();
    store.setSidebarMode("genres");
    store.loadAlbumsForGenre(genre);
  };

  return (
    <div className="now-playing">
      {/* Visible area — exactly fills the panel */}
      <div className="np-visible" style={panelHeight ? { height: panelHeight } : undefined}>
      {/* === TOP: Artist, Album, Year === */}
      <div className="np-top">
        <div className="np-header">
          <div className="np-artist np-clickable" onClick={handleArtistClick}>
            {hasTrackArtist
              ? `${artistName} (${track.trackArtist})`
              : artistName}
          </div>
          <div className="np-album-row">
            <span className="np-album-title np-clickable" onClick={handleAlbumClick}>{albumTitle}</span>
            <button
              className={`np-fav-btn${albumFav ? " active" : ""}`}
              onClick={handleAlbumFavToggle}
            >
              {albumFav ? "\u2605" : "\u2606"}
            </button>
          </div>
          {year && <div className="np-year np-clickable" onClick={handleYearClick}>{year}</div>}
        </div>
      </div>

      {/* === MIDDLE: Art, track, waveform, transport === */}
      <div className="np-middle">
        <div className="np-art-wrapper">
          <div
            className="np-art-container"
            onClick={toggleLyrics}
          >
            {artSrc && !artErr ? (
              <img
                className="np-art"
                src={artSrc}
                alt={albumTitle}
                crossOrigin="anonymous"
                onLoad={handleArtLoad}
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
        </div>

        <VolumeSlider value={volume} onChange={changeVolume} />

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

        <WaveformSeekBar />

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
      </div>

      {/* === BOTTOM: Genres, studio/codec, queue chevron === */}
      <div className="np-bottom">
        <div className="np-footer">
          <FlowLayout genres={currentGenres} onGenreClick={handleGenreClick} />
          {(studio || codec) && (
            <div className="np-meta-row">
              {studio && <span className="np-studio">{studio}</span>}
              {codec && <span className="np-format">{codec}</span>}
            </div>
          )}
        </div>
        <div className="np-queue-chevron">{"\u203A"}</div>
      </div>
      </div>

      {/* === Queue: below the fold === */}
      <QueueView />
    </div>
  );
}
