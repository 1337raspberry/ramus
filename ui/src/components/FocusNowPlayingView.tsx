import { usePlaybackStore } from "../stores/playbackStore";
import { ART_SIZE } from "../lib/commands";
import { togglePlayPause, nextTrack, previousTrack } from "../lib/commands";
import { useArtUrl } from "../lib/useArtUrl";
import { useQueuePanel } from "../lib/useQueuePanel";
import { useNowPlayingActions } from "../lib/useNowPlayingActions";
import WaveformSeekBar from "./WaveformSeekBar";
import VolumeSlider from "./VolumeSlider";
import FlowLayout from "./FlowLayout";
import LyricsOverlay from "./LyricsOverlay";
import QueueView from "./QueueView";
import FocusVisualizer from "./FocusVisualizer";
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
  IconClose,
} from "./Icons";

interface Props {
  onOpenEQ?: () => void;
}

/**
 * Full-screen "focus" Now Playing view. Mounted as an overlay from App.tsx
 * when `playbackStore.isFocusMode === true`.
 *
 * Layout: FocusVisualizer renders as a full-window background layer behind
 * everything (its curve drapes down from the very top edge). On top of that
 * sits a two-column grid, offset from the top by 32 px to clear the window
 * drag region — left panel holds the large album art with artist/album/year
 * anchored below, right panel holds track title + waveform + transport +
 * volume + genres + codec, plus an expandable queue that follows the same
 * wheel-down-to-reveal pattern as the compact DetailColumn.
 *
 * Reuses existing components and store actions wherever possible. Metadata
 * clicks (artist, album, year, genre) exit focus mode and navigate in the
 * main layout. Favourite toggles route through libraryStore per CLAUDE.md.
 */
export default function FocusNowPlayingView({ onOpenEQ }: Props) {
  const status = usePlaybackStore((s) => s.status);
  const toggleLyrics = usePlaybackStore((s) => s.toggleLyrics);
  const currentGenres = usePlaybackStore((s) => s.currentGenres);
  const volume = usePlaybackStore((s) => s.volume);
  const changeVolume = usePlaybackStore((s) => s.changeVolume);
  const toggleFocusMode = usePlaybackStore((s) => s.toggleFocusMode);

  // Navigation handlers exit focus mode when triggered (artist/album/year/
  // genre clicks), so pass toggleFocusMode as the onNavigate callback.
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
  } = useNowPlayingActions({ onNavigate: toggleFocusMode });

  const queue = useQueuePanel();

  const thumb = track?.thumb ?? nowPlayingAlbum?.thumb ?? null;
  // LARGE tier — shares cache with the compact panel + SuggestionView.
  // Palette / accent colour is already being set by the compact
  // NowPlayingView (still mounted underneath, just visually hidden), so we
  // don't re-extract here.
  const { artSrc, artErr, setArtErr } = useArtUrl(thumb, ART_SIZE.LARGE);

  if (!track) return null;

  const albumTitle = track.albumTitle;
  const artistName = track.artistName;

  return (
    <div className="focus-overlay">
      {/* Visualiser renders as a full-window background layer behind the art
       * and controls, draping from the very top of the frame. */}
      <FocusVisualizer />

      <div className="focus-body">
        {/* Left 50%: large album art with artist/album/year anchored below */}
        <div className="focus-art-panel">
          <div className="focus-art-wrapper">
            <div className="focus-art-container" onClick={toggleLyrics}>
              {artSrc && !artErr ? (
                <img
                  className="focus-art"
                  src={artSrc}
                  alt={albumTitle}
                  onError={() => setArtErr(true)}
                />
              ) : (
                <div className="focus-art-placeholder">
                  <IconMusicNote />
                </div>
              )}
              <LyricsOverlay />
            </div>
          </div>

          <div className="focus-art-meta">
            <div className="focus-artist np-clickable" onClick={handleArtistClick}>
              {hasTrackArtist ? `${artistName} (${track.trackArtist})` : artistName}
            </div>
            <div className="focus-album-row">
              <span className="focus-album-title np-clickable" onClick={handleAlbumClick}>
                {albumTitle}
              </span>
              {year && (
                <span className="focus-year np-clickable" onClick={handleYearClick}>
                  ({year})
                </span>
              )}
              <button
                className={`np-fav-btn${albumFav ? " active" : ""}`}
                onClick={handleAlbumFavToggle}
                title="Favourite album"
              >
                {albumFav ? <IconStarFilled /> : <IconStarEmpty />}
              </button>
            </div>
          </div>
        </div>

        {/* Right 50%: track, waveform, transport, volume, genres, queue */}
        <div
          className={`focus-controls-panel${queue.open ? " queue-open" : ""}`}
          onWheel={queue.onWheel}
          onScroll={queue.onScroll}
        >
          <div className="focus-controls-main">
            <div className="focus-track-row">
              <span className="focus-track-title">{track.title}</span>
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
                title="Favourite track"
              >
                {trackFav ? <IconStarFilled /> : <IconStarEmpty />}
              </button>
              <button
                className="focus-close-btn"
                onClick={toggleFocusMode}
                title="Exit focus mode (Esc)"
              >
                <IconClose size={14} />
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

            <VolumeSlider value={volume} onChange={changeVolume} />

            <div className="focus-footer">
              <FlowLayout genres={currentGenres} onGenreClick={handleGenreClick} />
              {(studio || codec) && (
                <div className="np-meta-row">
                  {studio && <span className="np-studio">{studio}</span>}
                  {codec && <span className="np-format">{codec}</span>}
                </div>
              )}
            </div>

            <button
              className={`np-queue-toggle${queue.open ? " expanded" : ""}`}
              onClick={queue.toggle}
              title="Up Next"
            >
              <IconChevronDown />
            </button>
          </div>

          {queue.open && (
            <div className="focus-queue-slot">
              <QueueView />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
