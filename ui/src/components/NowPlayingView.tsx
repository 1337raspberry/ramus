import { useCallback, useRef } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { ART_SIZE, setAlbumPalette } from "../lib/commands";
import { extractPalette, accentFromPalette, blurColorsFromPalette } from "../lib/vibrantColor";
import { useArtUrl } from "../lib/useArtUrl";
import { useNowPlayingActions } from "../lib/useNowPlayingActions";
import WaveformSeekBar from "./WaveformSeekBar";
import VolumeSlider from "./VolumeSlider";
import FlowLayout from "./FlowLayout";
import LyricsOverlay from "./LyricsOverlay";
import QueueView from "./QueueView";
import MarqueeText from "./MarqueeText";
import { togglePlayPause, nextTrack, previousTrack } from "../lib/commands";
import {
  IconStarFilled,
  IconStarEmpty,
  IconMusicNote,
  IconEqualizer,
  IconPrevious,
  IconPause,
  IconPlay,
  IconNext,
  IconChevronDown,
  IconExpand,
} from "./Icons";

interface NowPlayingProps {
  onOpenEQ?: () => void;
  panelHeight?: number;
  showQueue: boolean;
  onToggleQueue: () => void;
}

export default function NowPlayingView({
  onOpenEQ,
  panelHeight,
  showQueue,
  onToggleQueue,
}: NowPlayingProps) {
  const status = usePlaybackStore((s) => s.status);
  const toggleLyrics = usePlaybackStore((s) => s.toggleLyrics);
  const currentGenres = usePlaybackStore((s) => s.currentGenres);
  const volume = usePlaybackStore((s) => s.volume);
  const changeVolume = usePlaybackStore((s) => s.changeVolume);

  const {
    track,
    nowPlayingAlbum,
    hasTrackArtist,
    year,
    studio,
    codec,
    albumFav,
    trackFav,
    handleAlbumFavToggle,
    handleTrackFavToggle,
    handleArtistClick,
    handleAlbumClick,
    handleYearClick,
    handleGenreClick,
  } = useNowPlayingActions();

  const thumb = track?.thumb ?? nowPlayingAlbum?.thumb ?? null;
  // LARGE tier — shared with the focus view and SuggestionView so entering
  // focus mode is an instant cache hit and the compact panel stays crisp
  // on HiDPI displays.
  const { artSrc, artErr, setArtErr } = useArtUrl(thumb, ART_SIZE.LARGE);
  const lastAccentThumb = useRef<string | null>(null);

  const handleArtLoad = useCallback(
    (e: React.SyntheticEvent<HTMLImageElement>) => {
      const img = e.currentTarget;
      if (lastAccentThumb.current === thumb) return;
      lastAccentThumb.current = thumb;
      const capturedThumb = thumb;
      // Skip palette extraction if already cached from getAlbumColors
      const existing = usePlaybackStore.getState().vibrantPalette;
      if (existing) {
        const [r, g, b] = accentFromPalette(existing);
        document.documentElement.style.setProperty("--accent-r", String(r));
        document.documentElement.style.setProperty("--accent-g", String(g));
        document.documentElement.style.setProperty("--accent-b", String(b));
        return;
      }
      extractPalette(img).then((palette) => {
        if (!palette || lastAccentThumb.current !== capturedThumb) return;
        const [r, g, b] = accentFromPalette(palette);
        document.documentElement.style.setProperty("--accent-r", String(r));
        document.documentElement.style.setProperty("--accent-g", String(g));
        document.documentElement.style.setProperty("--accent-b", String(b));
        const blurColors = blurColorsFromPalette(palette);
        usePlaybackStore.setState({ vibrantPalette: palette, ultraBlurColors: blurColors });
        if (track?.albumKey) {
          setAlbumPalette(track.albumKey, palette).catch(() => {});
        }
      });
    },
    [thumb, track?.albumKey],
  );

  if (!track) return null;

  const albumTitle = track.albumTitle;
  const artistName = track.artistName;

  const handleEnterFocusMode = () => {
    usePlaybackStore.getState().toggleFocusMode();
  };

  return (
    <div className="now-playing">
      {/* Visible area — exactly fills the panel */}
      <div className="np-visible" style={panelHeight ? { height: panelHeight } : undefined}>
        {/* === TOP: Artist, Album, Year === */}
        <div className="np-top">
          <div className="np-header">
            <MarqueeText className="np-artist np-clickable" onClick={handleArtistClick}>
              {hasTrackArtist ? `${artistName} (${track.trackArtist})` : artistName}
            </MarqueeText>
            <div className="np-album-row">
              <MarqueeText className="np-album-title np-clickable" onClick={handleAlbumClick}>
                {albumTitle}
              </MarqueeText>
              <button
                className={`np-fav-btn${albumFav ? " active" : ""}`}
                onClick={handleAlbumFavToggle}
              >
                {albumFav ? <IconStarFilled /> : <IconStarEmpty />}
              </button>
              <button
                className="np-focus-btn"
                onClick={handleEnterFocusMode}
                title="Focus mode (⇧⌘N)"
              >
                <IconExpand size={14} />
              </button>
            </div>
            {year && (
              <div className="np-year np-clickable" onClick={handleYearClick}>
                {year}
              </div>
            )}
          </div>
        </div>

        {/* === MIDDLE: Art, track, waveform, transport === */}
        <div className="np-middle">
          <div className="np-art-wrapper">
            <div className="np-art-container" onClick={toggleLyrics}>
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
                <div className="np-art-placeholder">
                  <IconMusicNote />
                </div>
              )}
              <LyricsOverlay />
            </div>
          </div>

          <VolumeSlider value={volume} onChange={changeVolume} />

          <div className="np-track-row">
            <MarqueeText className="np-track-title">{track.title}</MarqueeText>
            {onOpenEQ && (
              <button className="np-eq-btn" onClick={onOpenEQ} title="Equalizer">
                <IconEqualizer />
              </button>
            )}
            <button
              className={`np-fav-btn${trackFav ? " active" : ""}`}
              onClick={(e) => {
                e.stopPropagation();
                handleTrackFavToggle();
              }}
            >
              {trackFav ? <IconStarFilled /> : <IconStarEmpty />}
            </button>
          </div>

          <WaveformSeekBar />

          <div className="np-transport">
            <button className="np-transport-btn" onClick={() => previousTrack()}>
              <IconPrevious />
            </button>
            <button className="np-transport-btn np-play-btn" onClick={() => togglePlayPause()}>
              {status === "playing" ? <IconPause /> : <IconPlay />}
            </button>
            <button className="np-transport-btn" onClick={() => nextTrack()}>
              <IconNext />
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
          <button
            className={`np-queue-toggle${showQueue ? " expanded" : ""}`}
            onClick={onToggleQueue}
          >
            <IconChevronDown />
          </button>
        </div>
      </div>

      {/* === Queue: toggled by chevron === */}
      {showQueue && <QueueView />}
    </div>
  );
}
