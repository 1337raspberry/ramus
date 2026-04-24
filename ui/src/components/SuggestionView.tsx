import { useCallback, useEffect, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import {
  ART_SIZE,
  getArtUrl,
  getAlbumColors,
  getAlbumGenres,
  getTracksForAlbum,
  setAlbumPalette,
} from "../lib/commands";
import { extractPalette, accentFromPalette, blurColorsFromPalette } from "../lib/vibrantColor";
import { applyAccent } from "../lib/accent";
import { formatCodec } from "../lib/format";
import { countryToFlag } from "../lib/countryFlag";
import FlowLayout from "./FlowLayout";
import { IconMusicNote } from "./Icons";

export default function SuggestionView() {
  const album = useLibraryStore((s) => s.suggestion);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const clearSuggestion = useLibraryStore((s) => s.clearSuggestion);
  const status = usePlaybackStore((s) => s.status);

  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artErr, setArtErr] = useState(false);
  const [genres, setGenres] = useState<string[]>([]);
  const [codec, setCodec] = useState<string | null>(null);

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
    if (!album) {
      setGenres([]);
      return;
    }
    // Use album.genres when present; otherwise fetch.
    if (album.genres.length) {
      setGenres(album.genres);
    } else {
      let cancelled = false;
      getAlbumGenres(album.ratingKey)
        .then((g) => {
          if (!cancelled) setGenres(g);
        })
        .catch(() => {
          if (!cancelled) setGenres([]);
        });
      return () => {
        cancelled = true;
      };
    }
  }, [album]);

  useEffect(() => {
    if (!album) {
      setCodec(null);
      return;
    }
    let cancelled = false;
    getTracksForAlbum(album.ratingKey)
      .then((tracks) => {
        if (!cancelled && tracks.length) {
          setCodec(formatCodec(tracks[0].codec, tracks[0].bitrate));
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [album]);

  // Prime the store with a cached vibrant palette. Dynamic extraction
  // in handleArtLoad is the source of truth; the legacy API-sourced
  // ultraBlurColors path is intentionally skipped. Depends only on
  // `album`: SuggestionView only renders while stopped, so a `status`
  // guard would just cause palette flashes on status flicker.
  useEffect(() => {
    if (!album) return;
    // Clear the previous suggestion's palette so handleArtLoad falls
    // through to extractPalette() for the new image.
    usePlaybackStore.setState({ vibrantPalette: null, ultraBlurColors: null });
    getAlbumColors(album.ratingKey)
      .then((result) => {
        if (result.palette) {
          usePlaybackStore.setState({
            vibrantPalette: result.palette,
            ultraBlurColors: blurColorsFromPalette(result.palette),
          });
        }
      })
      .catch(() => {});
  }, [album]);

  const handleArtLoad = useCallback(
    (e: React.SyntheticEvent<HTMLImageElement>) => {
      if (status !== "stopped") return;
      // Skip when a palette was already loaded from the DB cache.
      const existing = usePlaybackStore.getState().vibrantPalette;
      if (existing) {
        const [r, g, b] = accentFromPalette(existing);
        applyAccent(r, g, b);
        return;
      }
      extractPalette(e.currentTarget).then((palette) => {
        if (!palette) return;
        const [r, g, b] = accentFromPalette(palette);
        applyAccent(r, g, b);
        const blurColors = blurColorsFromPalette(palette);
        usePlaybackStore.setState({ vibrantPalette: palette, ultraBlurColors: blurColors });
        if (album) {
          setAlbumPalette(album.ratingKey, palette).catch(() => {});
        }
      });
    },
    [status, album],
  );

  const handleClick = useCallback(() => {
    if (!album) return;
    playAlbum(album);
    clearSuggestion();
  }, [album, playAlbum, clearSuggestion]);

  const handleGenreClick = useCallback((genre: string) => {
    useLibraryStore.getState().clearSuggestion();
    useLibraryStore.getState().selectGenreByName(genre);
  }, []);

  if (!album) return null;

  const yearStr = album.year ? ` (${album.year})` : "";

  return (
    <div className="suggestion-view">
      <div className="suggestion-card" onClick={handleClick}>
        <div className="suggestion-art-wrapper">
          {artSrc && !artErr ? (
            <img
              className="suggestion-art"
              src={artSrc}
              alt={album.title}
              crossOrigin="anonymous"
              onLoad={handleArtLoad}
              onError={() => setArtErr(true)}
            />
          ) : (
            <div className="suggestion-art-placeholder">
              <IconMusicNote />
            </div>
          )}
        </div>
        <div className="suggestion-info">
          <div className="suggestion-title">
            {album.artistName}
            {(() => {
              const flag = album.artistCountry ? countryToFlag(album.artistCountry) : null;
              return flag ? (
                <span className="adv-country-flag" title={album.artistCountry!}>
                  {flag}
                </span>
              ) : null;
            })()}{" "}
            &mdash; {album.title}
            {yearStr}
          </div>
        </div>
      </div>
      <div className="suggestion-genres" onClick={(e) => e.stopPropagation()}>
        <FlowLayout genres={genres} onGenreClick={handleGenreClick} />
      </div>
      <div className="suggestion-meta">
        {album.studio && <span className="suggestion-studio">{album.studio}</span>}
        {codec && <span className="suggestion-codec">{codec}</span>}
      </div>
    </div>
  );
}
