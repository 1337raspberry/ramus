import { useCallback, useEffect, useRef, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import {
  ART_SIZE,
  getArtUrl,
  getAlbumColors,
  getAlbumGenres,
  setAlbumPalette,
} from "../lib/commands";
import { extractPalette, accentFromPalette, blurColorsFromPalette } from "../lib/vibrantColor";
import { applyAccent } from "../lib/accent";
import { countryToFlag } from "../lib/countryFlag";
import {
  IconMusicNote,
  IconShuffle,
  IconChevronLeft,
  IconFilter,
  IconStopwatch,
} from "../components/Icons";
import FlowLayout from "../components/FlowLayout";
import MobileFilterPanel from "./MobileFilterPanel";
import MobileDurationPicker from "./MobileDurationPicker";
import { hasActiveFilters, SUGGEST_TOLERANCE_MIN } from "../stores/libraryStore";
import { useGenreInfoStore } from "../stores/genreInfoStore";

function formatTarget(minutes: number): string {
  return `${Math.floor(minutes / 60)}h ${String(minutes % 60).padStart(2, "0")}m`;
}

interface Props {
  onClose: () => void;
  onPlay: () => void;
}

export default function MobileSuggestion({ onClose, onPlay }: Props) {
  const album = useLibraryStore((s) => s.suggestion);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const loadSuggestion = useLibraryStore((s) => s.loadSuggestion);
  const clearSuggestion = useLibraryStore((s) => s.clearSuggestion);
  const selectGenreByName = useLibraryStore((s) => s.selectGenreByName);
  const loadAlbumsForArtistName = useLibraryStore((s) => s.loadAlbumsForArtistName);
  const showArtistFlags = useSettingsStore((s) => s.showArtistFlags);

  const albumFilters = useLibraryStore((s) => s.albumFilters);
  const targetMinutes = useLibraryStore((s) => s.suggestionTargetMinutes);
  const suggestionMissed = useLibraryStore((s) => s.suggestionMissed);
  const [showFilter, setShowFilter] = useState(false);
  const [showDuration, setShowDuration] = useState(false);
  const durationBtnRef = useRef<HTMLButtonElement>(null);
  const filterActive = hasActiveFilters(albumFilters);
  const targetActive = targetMinutes != null;

  const openGenreInfo = useGenreInfoStore((s) => s.open);

  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artErr, setArtErr] = useState(false);
  const [genres, setGenres] = useState<string[]>([]);

  // The miss message describes the query-time state; re-run the query when
  // filters change so it can't keep claiming "no albums match your filters"
  // after the filters were relaxed.
  useEffect(() => {
    if (useLibraryStore.getState().suggestionMissed) loadSuggestion();
  }, [albumFilters, loadSuggestion]);

  useEffect(() => {
    if (!album?.thumb) {
      setArtSrc(null);
      return;
    }
    setArtErr(false);
    setArtSrc(null);
    let cancelled = false;
    getArtUrl(album.thumb, ART_SIZE.LARGE)
      .then((url) => {
        if (!cancelled) setArtSrc(url);
      })
      .catch(() => {
        if (!cancelled) setArtErr(true);
      });
    return () => {
      cancelled = true;
    };
  }, [album?.thumb]);

  useEffect(() => {
    if (!album) return;
    if (album.genres.length) {
      setGenres(album.genres);
    } else {
      getAlbumGenres(album.ratingKey)
        .then(setGenres)
        .catch(() => {});
    }
    // Only tint the background when nothing is playing — the now-playing
    // track's palette always takes precedence.
    const isPlaying = !!usePlaybackStore.getState().currentTrack;
    if (!isPlaying) {
      usePlaybackStore.setState({ vibrantPalette: null, ultraBlurColors: null });
      getAlbumColors(album.ratingKey)
        .then((result) => {
          if (usePlaybackStore.getState().currentTrack) return;
          if (result.palette) {
            usePlaybackStore.setState({
              vibrantPalette: result.palette,
              ultraBlurColors: blurColorsFromPalette(result.palette),
            });
          }
        })
        .catch(() => {});
    }
  }, [album]);

  const handleArtLoad = useCallback(
    (e: React.SyntheticEvent<HTMLImageElement>) => {
      if (usePlaybackStore.getState().currentTrack) return;
      const existing = usePlaybackStore.getState().vibrantPalette;
      if (existing) {
        const [r, g, b] = accentFromPalette(existing);
        applyAccent(r, g, b);
        return;
      }
      extractPalette(e.currentTarget).then((palette) => {
        if (!palette || usePlaybackStore.getState().currentTrack) return;
        const [r, g, b] = accentFromPalette(palette);
        applyAccent(r, g, b);
        const blur = blurColorsFromPalette(palette);
        usePlaybackStore.setState({ vibrantPalette: palette, ultraBlurColors: blur });
        if (album) setAlbumPalette(album.ratingKey, palette).catch(() => {});
      });
    },
    [album],
  );

  const handleClose = () => {
    clearSuggestion();
    onClose();
  };

  const renderHeader = () => (
    <header className="mobile-header mobile-header-5col">
      <button className="mobile-header-circle" onClick={handleClose} aria-label="Back">
        <IconChevronLeft size={22} />
      </button>
      <button
        ref={durationBtnRef}
        className={`mobile-header-circle${targetActive ? " accent" : ""}`}
        onClick={() => setShowDuration((v) => !v)}
        aria-label="Target album length"
      >
        <IconStopwatch size={20} />
        {targetActive && <span className="mobile-filter-dot" />}
      </button>
      <div className="mobile-header-title">{targetActive ? formatTarget(targetMinutes!) : " "}</div>
      <button
        className={`mobile-header-circle${filterActive ? " accent" : ""}`}
        onClick={() => setShowFilter(true)}
        aria-label="Filter suggestions"
      >
        <IconFilter size={18} />
        {filterActive && <span className="mobile-filter-dot" />}
      </button>
      <button className="mobile-header-circle" onClick={loadSuggestion} aria-label="New suggestion">
        <IconShuffle size={22} />
      </button>
    </header>
  );

  const overlays = (
    <>
      {showFilter && <MobileFilterPanel onDismiss={() => setShowFilter(false)} />}
      {showDuration && (
        <MobileDurationPicker anchorRef={durationBtnRef} onDismiss={() => setShowDuration(false)} />
      )}
    </>
  );

  // A "missed" query keeps the stale `suggestion` (so desktop is unaffected),
  // so check the miss flag before falling through to the album render.
  if (suggestionMissed) {
    return (
      <div className="mobile-screen mobile-suggestion">
        {renderHeader()}
        {overlays}
        <div className="mobile-empty">
          {targetActive
            ? `No albums within ±${SUGGEST_TOLERANCE_MIN} min of ${formatTarget(targetMinutes!)}. Try another length or adjust your filters.`
            : filterActive
              ? "No albums match your current filters."
              : "No albums to suggest yet."}
        </div>
      </div>
    );
  }

  if (!album) {
    return (
      <div className="mobile-screen mobile-suggestion">
        {renderHeader()}
        {overlays}
        <div className="mobile-empty">Loading suggestion...</div>
      </div>
    );
  }

  return (
    <div className="mobile-screen mobile-suggestion">
      {renderHeader()}
      {overlays}

      <div className="mobile-suggestion-body">
        <button
          className="mobile-suggestion-card"
          onClick={() => {
            playAlbum(album);
            clearSuggestion();
            onPlay();
          }}
        >
          {artSrc && !artErr ? (
            <img
              src={artSrc}
              alt={album.title}
              crossOrigin="anonymous"
              onLoad={handleArtLoad}
              onError={() => setArtErr(true)}
            />
          ) : (
            <div className="mobile-suggestion-art-ph">
              <IconMusicNote size={64} />
            </div>
          )}
        </button>
        <div className="mobile-suggestion-title">
          {album.title}
          {album.year ? <span className="mobile-suggestion-year"> · {album.year}</span> : null}
        </div>
        <button
          type="button"
          className="mobile-suggestion-artist"
          onClick={() => {
            onClose();
            clearSuggestion();
            loadAlbumsForArtistName(album.artistName);
          }}
        >
          {album.artistName}
          {(() => {
            const flag =
              showArtistFlags && album.artistCountry ? countryToFlag(album.artistCountry) : null;
            return flag ? (
              <span className="adv-country-flag" title={album.artistCountry!}>
                {flag}
              </span>
            ) : null;
          })()}
        </button>
        {genres.length > 0 && (
          <div className="mobile-suggestion-genres">
            <FlowLayout
              genres={genres}
              onGenreClick={(g) => {
                onClose();
                clearSuggestion();
                selectGenreByName(g);
              }}
              onGenreLongPress={openGenreInfo}
            />
          </div>
        )}
      </div>
    </div>
  );
}
