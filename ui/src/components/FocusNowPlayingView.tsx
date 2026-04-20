import { useCallback, useEffect, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useLibraryStore } from "../stores/libraryStore";
import { ART_SIZE, setAlbumPalette, getAlbumGenres } from "../lib/commands";
import { togglePlayPause, nextTrack, previousTrack } from "../lib/commands";
import { extractPalette, accentFromPalette, blurColorsFromPalette } from "../lib/vibrantColor";
import { applyAccent } from "../lib/accent";
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
 * Full-screen Now Playing overlay. Mounted from App.tsx when
 * `playbackStore.isFocusMode === true`.
 *
 * Layout: FocusVisualizer paints a full-window background layer (drapes
 * from the top edge). A two-column grid sits on top, offset 32px from
 * the top to clear the window drag region. Left: album art with
 * artist/album/year anchored below. Right: track title, waveform,
 * transport, volume, genres, codec, plus an expandable queue that reuses
 * the wheel-down-to-reveal pattern from DetailColumn.
 *
 * Metadata clicks (artist/album/year/genre) exit focus mode and
 * navigate in the main layout. Favourite toggles route through
 * libraryStore (see CLAUDE.md).
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

  const suggestion = useLibraryStore((s) => s.suggestion);
  const loadSuggestion = useLibraryStore((s) => s.loadSuggestion);
  const clearSuggestion = useLibraryStore((s) => s.clearSuggestion);
  const playAlbum = useLibraryStore((s) => s.playAlbum);

  // Pass toggleFocusMode so artist/album/year/genre clicks exit focus mode.
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
  // LARGE tier shares cache with the compact panel and SuggestionView.
  // The compact NowPlayingView (still mounted, visually hidden) already
  // sets palette and accent; do not re-extract here.
  const { artSrc, artErr, setArtErr } = useArtUrl(thumb, ART_SIZE.LARGE);

  // True when this focus session loaded the suggestion, as opposed to
  // one already present from the sidebar "Feelin Lucky" button.
  const ownsSuggestionRef = useRef(false);

  // Only clear focus-originated suggestions on unmount so sidebar-loaded
  // suggestions survive exiting focus mode.
  useEffect(() => {
    return () => {
      if (ownsSuggestionRef.current) {
        useLibraryStore.getState().clearSuggestion();
      }
    };
  }, []);

  // Auto-load a suggestion on playback stop (queue exhausted). The
  // `playedRef` guard prevents a second loadSuggestion in the async
  // window between clearing the previous suggestion and the first track
  // event for the user-accepted one.
  const playedRef = useRef(false);
  useEffect(() => {
    if (track) {
      // Reset guard so the next queue-end triggers a fresh suggestion.
      playedRef.current = false;
      return;
    }
    if (status === "stopped" && !suggestion && !playedRef.current) {
      ownsSuggestionRef.current = true;
      loadSuggestion();
    }
  }, [track, status, suggestion, loadSuggestion]);

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
        applyAccent(r, g, b);
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
    // Do not clearSuggestion() first: a null window would re-trigger
    // the auto-load effect. loadSuggestion overwrites atomically when
    // the fetch resolves.
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

  // Stopped state: show a suggestion card instead of a blank screen.
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
      {/* Visualiser is a full-window background layer behind art and
       * controls, draping from the top edge. Gated on
       * `visualizerMode !== "off"` and unmounted (not CSS-hidden) so
       * the RAF loop stops in "off" mode. The viz reads its own mode
       * via `usePlaybackStore.getState()` inside the RAF loop, so
       * cycling bars ↔ line does not remount the canvas. */}
      {visualizerMode !== "off" && <FocusVisualizer />}

      <div className="focus-body">
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
