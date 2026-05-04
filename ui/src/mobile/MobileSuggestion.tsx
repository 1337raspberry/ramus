import { useCallback, useEffect, useState } from "react";
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
import { IconMusicNote, IconShuffle, IconChevronLeft, IconFilter } from "../components/Icons";
import FlowLayout from "../components/FlowLayout";
import MobileFilterPanel from "./MobileFilterPanel";
import { hasActiveFilters } from "../stores/libraryStore";

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
  const [showFilter, setShowFilter] = useState(false);
  const filterActive = hasActiveFilters(albumFilters);

  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artErr, setArtErr] = useState(false);
  const [genres, setGenres] = useState<string[]>([]);

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

  if (!album) {
    return (
      <div className="mobile-screen mobile-suggestion">
        <header className="mobile-header mobile-header-4col">
          <button className="mobile-header-circle" onClick={handleClose} aria-label="Back">
            <IconChevronLeft size={22} />
          </button>
          <div className="mobile-header-title"> </div>
          <button
            className={`mobile-header-circle${filterActive ? " accent" : ""}`}
            onClick={() => setShowFilter(true)}
            aria-label="Filter suggestions"
          >
            <IconFilter size={18} />
            {filterActive && <span className="mobile-filter-dot" />}
          </button>
          <button
            className="mobile-header-circle"
            onClick={loadSuggestion}
            aria-label="New suggestion"
          >
            <IconShuffle size={22} />
          </button>
        </header>
        {showFilter && <MobileFilterPanel onDismiss={() => setShowFilter(false)} />}
        <div className="mobile-empty">Loading suggestion...</div>
      </div>
    );
  }

  return (
    <div className="mobile-screen mobile-suggestion">
      <header className="mobile-header mobile-header-4col">
        <button className="mobile-header-circle" onClick={handleClose} aria-label="Back">
          <IconChevronLeft size={22} />
        </button>
        <div className="mobile-header-title"> </div>
        <button
          className={`mobile-header-circle${filterActive ? " accent" : ""}`}
          onClick={() => setShowFilter(true)}
          aria-label="Filter suggestions"
        >
          <IconFilter size={18} />
          {filterActive && <span className="mobile-filter-dot" />}
        </button>
        <button
          className="mobile-header-circle"
          onClick={loadSuggestion}
          aria-label="New suggestion"
        >
          <IconShuffle size={22} />
        </button>
      </header>
      {showFilter && <MobileFilterPanel onDismiss={() => setShowFilter(false)} />}

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
            />
          </div>
        )}
      </div>
    </div>
  );
}
