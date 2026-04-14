import { useCallback, useEffect, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useLibraryStore } from "../stores/libraryStore";
import { ART_SIZE, setAlbumPalette, getAlbumGenres } from "../lib/commands";
import { togglePlayPause, nextTrack, previousTrack } from "../lib/commands";
import { extractPalette, accentFromPalette, blurColorsFromPalette } from "../lib/vibrantColor";
import { useArtUrl } from "../lib/useArtUrl";
import { useQueuePanel } from "../lib/useQueuePanel";
import { useNowPlayingActions } from "../lib/useNowPlayingActions";
import WaveformSeekBar from "./WaveformSeekBar";
import VolumeSlider from "./VolumeSlider";
import FlowLayout from "./FlowLayout";
import LyricsOverlay from "./LyricsOverlay";
import QueueView from "./QueueView";
import FocusVisualizer from "./FocusVisualizer";
import MarqueeText from "./MarqueeText";
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
  IconWave,
  IconShuffle,
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
  const visualizerMode = usePlaybackStore((s) => s.visualizerMode);
  const cycleVisualizerMode = usePlaybackStore((s) => s.cycleVisualizerMode);

  // Suggestion state for when the queue ends
  const suggestion = useLibraryStore((s) => s.suggestion);
  const loadSuggestion = useLibraryStore((s) => s.loadSuggestion);
  const clearSuggestion = useLibraryStore((s) => s.clearSuggestion);
  const playAlbum = useLibraryStore((s) => s.playAlbum);

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

  // Track whether THIS focus session loaded a suggestion (vs one that
  // already existed from the sidebar "Feelin Lucky" button).
  const ownsSuggestionRef = useRef(false);

  // Clear only focus-originated suggestions on unmount so a sidebar
  // suggestion isn't wiped when the user exits focus mode.
  useEffect(() => {
    return () => {
      if (ownsSuggestionRef.current) {
        useLibraryStore.getState().clearSuggestion();
      }
    };
  }, []);

  // Auto-load a suggestion when playback stops (queue exhausted).
  // The `played` ref prevents re-loading after the user accepts a
  // suggestion — playAlbum is async, so there's a brief window where
  // track is still null and suggestion was just cleared, which would
  // otherwise trigger another loadSuggestion before the first track
  // event arrives.
  const playedRef = useRef(false);
  useEffect(() => {
    if (track) {
      // Track arrived (user played a suggestion or new queue loaded).
      // Reset the guard so the next queue-end triggers a fresh suggestion.
      playedRef.current = false;
      return;
    }
    if (status === "stopped" && !suggestion && !playedRef.current) {
      ownsSuggestionRef.current = true;
      loadSuggestion();
    }
  }, [track, status, suggestion, loadSuggestion]);

  // Art + palette + genres for the suggestion card when stopped
  const suggestionThumb = suggestion?.thumb ?? null;
  const {
    artSrc: suggestionArtSrc,
    artErr: suggestionArtErr,
    setArtErr: setSuggestionArtErr,
  } = useArtUrl(suggestionThumb, ART_SIZE.LARGE);

  const [suggestionGenres, setSuggestionGenres] = useState<string[]>([]);
  useEffect(() => {
    if (!suggestion) {
      setSuggestionGenres([]);
      return;
    }
    if (suggestion.genres.length) {
      setSuggestionGenres(suggestion.genres);
    } else {
      let cancelled = false;
      getAlbumGenres(suggestion.ratingKey)
        .then((g) => {
          if (!cancelled) setSuggestionGenres(g);
        })
        .catch(() => {
          if (!cancelled) setSuggestionGenres([]);
        });
      return () => {
        cancelled = true;
      };
    }
  }, [suggestion]);

  const handleSuggestionArtLoad = useCallback(
    (e: React.SyntheticEvent<HTMLImageElement>) => {
      extractPalette(e.currentTarget).then((palette) => {
        if (!palette) return;
        const [r, g, b] = accentFromPalette(palette);
        document.documentElement.style.setProperty("--accent-r", String(r));
        document.documentElement.style.setProperty("--accent-g", String(g));
        document.documentElement.style.setProperty("--accent-b", String(b));
        const blurColors = blurColorsFromPalette(palette);
        usePlaybackStore.setState({ vibrantPalette: palette, ultraBlurColors: blurColors });
        if (suggestion) {
          setAlbumPalette(suggestion.ratingKey, palette).catch(() => {});
        }
      });
    },
    [suggestion],
  );

  const handleSuggestionPlay = useCallback(() => {
    if (!suggestion) return;
    playedRef.current = true;
    playAlbum(suggestion);
    clearSuggestion();
  }, [suggestion, playAlbum, clearSuggestion]);

  const handleSuggestionShuffle = useCallback(() => {
    if (playedRef.current) return;
    usePlaybackStore.setState({ vibrantPalette: null, ultraBlurColors: null });
    // Don't clearSuggestion() first — that would leave a !suggestion
    // window the auto-load effect reacts to. loadSuggestion overwrites
    // the old suggestion atomically when the fetch resolves.
    loadSuggestion();
  }, [loadSuggestion]);

  const handleSuggestionDismiss = useCallback(() => {
    clearSuggestion();
    toggleFocusMode();
  }, [clearSuggestion, toggleFocusMode]);

  const handleSuggestionGenreClick = useCallback(
    (genre: string) => {
      clearSuggestion();
      toggleFocusMode();
      useLibraryStore.getState().selectGenreByName(genre);
    },
    [clearSuggestion, toggleFocusMode],
  );

  // Stopped state — show a suggestion card instead of a blank screen
  if (!track) {
    return (
      <div className="focus-overlay">
        <div className="focus-stopped-view">
          {suggestion ? (
            <>
              <div className="focus-stopped-label">Up Next?</div>
              <div className="focus-stopped-card" onClick={handleSuggestionPlay}>
                <div className="focus-stopped-art-container">
                  {suggestionArtSrc && !suggestionArtErr ? (
                    <img
                      className="focus-art"
                      src={suggestionArtSrc}
                      alt={suggestion.title}
                      crossOrigin="anonymous"
                      onLoad={handleSuggestionArtLoad}
                      onError={() => setSuggestionArtErr(true)}
                    />
                  ) : (
                    <div className="focus-art-placeholder">
                      <IconMusicNote />
                    </div>
                  )}
                </div>
                <div className="focus-stopped-meta">
                  <div className="focus-stopped-title">
                    {suggestion.artistName} &mdash; {suggestion.title}
                    {suggestion.year ? ` (${suggestion.year})` : ""}
                  </div>
                </div>
              </div>
              {suggestionGenres.length > 0 && (
                <div className="focus-stopped-genres" onClick={(e) => e.stopPropagation()}>
                  <FlowLayout genres={suggestionGenres} onGenreClick={handleSuggestionGenreClick} />
                </div>
              )}
            </>
          ) : (
            <div className="focus-stopped-loading" />
          )}
          {suggestion && (
            <button
              className="focus-stopped-shuffle"
              onClick={handleSuggestionShuffle}
              title="Shuffle — pick another"
            >
              <IconShuffle size={16} />
            </button>
          )}
          <button
            className="focus-stopped-dismiss"
            onClick={handleSuggestionDismiss}
            title="Exit focus mode"
          >
            <IconClose size={16} />
          </button>
        </div>
      </div>
    );
  }

  const albumTitle = track.albumTitle;
  const artistName = track.artistName;

  return (
    <div className="focus-overlay">
      {/* Visualiser renders as a full-window background layer behind the
       * art and controls, draping from the very top of the frame. Gated
       * on `visualizerMode !== "off"` so the wave button in the track
       * row can cycle bars → line → off. Unmount (not CSS-hide) when
       * off so the RAF loop stops. The viz reads its own mode via
       * `usePlaybackStore.getState()` inside the RAF loop so switching
       * between bars and line doesn't remount the canvas. */}
      {visualizerMode !== "off" && <FocusVisualizer />}

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
            <MarqueeText className="focus-artist np-clickable" onClick={handleArtistClick}>
              {hasTrackArtist ? `${artistName} (${track.trackArtist})` : artistName}
            </MarqueeText>
            <div className="focus-album-row">
              <MarqueeText className="focus-album-title np-clickable" onClick={handleAlbumClick}>
                {albumTitle}
              </MarqueeText>
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
              <MarqueeText className="focus-track-title">{track.title}</MarqueeText>
              {onOpenEQ && (
                <button className="np-eq-btn" onClick={onOpenEQ} title="Equalizer">
                  <IconEqualizer />
                </button>
              )}
              <button
                className={`np-viz-btn${visualizerMode !== "off" ? " active" : ""}`}
                onClick={cycleVisualizerMode}
                title={
                  visualizerMode === "bars"
                    ? "Visualiser: bars — click for line mode"
                    : visualizerMode === "line"
                      ? "Visualiser: line — click to hide"
                      : "Visualiser: off — click to show bars"
                }
                aria-label={
                  visualizerMode === "bars"
                    ? "Visualiser: bars, click for line mode"
                    : visualizerMode === "line"
                      ? "Visualiser: line, click to hide"
                      : "Visualiser: off, click to show bars"
                }
              >
                <IconWave />
              </button>
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
