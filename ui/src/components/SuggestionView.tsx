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
import { formatCodec } from "../lib/format";
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
    // Use album.genres if available, otherwise fetch
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

  // Prime the store with a previously-extracted vibrant palette if we have
  // one cached in the DB. Skip the legacy API-sourced ultraBlurColors entirely
  // — dynamic extraction in handleArtLoad is the source of truth.
  useEffect(() => {
    if (!album || status !== "stopped") return;
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
  }, [album, status]);

  const handleArtLoad = useCallback(
    (e: React.SyntheticEvent<HTMLImageElement>) => {
      if (status !== "stopped") return;
      // Skip if palette was already loaded from the DB cache
      const existing = usePlaybackStore.getState().vibrantPalette;
      if (existing) {
        const [r, g, b] = accentFromPalette(existing);
        document.documentElement.style.setProperty("--accent-r", String(r));
        document.documentElement.style.setProperty("--accent-g", String(g));
        document.documentElement.style.setProperty("--accent-b", String(b));
        return;
      }
      extractPalette(e.currentTarget).then((palette) => {
        if (!palette) return;
        const [r, g, b] = accentFromPalette(palette);
        document.documentElement.style.setProperty("--accent-r", String(r));
        document.documentElement.style.setProperty("--accent-g", String(g));
        document.documentElement.style.setProperty("--accent-b", String(b));
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
    const store = useLibraryStore.getState();
    store.clearSuggestion();
    store.setSidebarMode("genres");
    store.loadAlbumsForGenre(genre);
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
            {album.artistName} &mdash; {album.title}
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
