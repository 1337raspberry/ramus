import { useCallback, useEffect, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { getArtUrl, getAlbumColors, getAlbumGenres, getTracksForAlbum } from "../lib/commands";
import { extractVibrantColor } from "../lib/vibrantColor";
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

  // Fetch art URL
  useEffect(() => {
    if (!album?.thumb) { setArtSrc(null); return; }
    setArtErr(false);
    setArtSrc(null);
    let cancelled = false;
    getArtUrl(album.thumb, 600)
      .then((url) => { if (!cancelled) setArtSrc(url); })
      .catch(() => { if (!cancelled) setArtErr(true); });
    return () => { cancelled = true; };
  }, [album?.thumb]);

  // Fetch genres
  useEffect(() => {
    if (!album) { setGenres([]); return; }
    // Use album.genres if available, otherwise fetch
    if (album.genres.length) {
      setGenres(album.genres);
    } else {
      let cancelled = false;
      getAlbumGenres(album.ratingKey)
        .then((g) => { if (!cancelled) setGenres(g); })
        .catch(() => { if (!cancelled) setGenres([]); });
      return () => { cancelled = true; };
    }
  }, [album]);

  // Fetch codec from first track
  useEffect(() => {
    if (!album) { setCodec(null); return; }
    let cancelled = false;
    getTracksForAlbum(album.ratingKey)
      .then((tracks) => {
        if (!cancelled && tracks.length) {
          setCodec(formatCodec(tracks[0].codec, tracks[0].bitrate));
        }
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [album]);

  // Set ultrablur + accent when nothing is playing
  useEffect(() => {
    if (!album || status !== "stopped") return;
    getAlbumColors(album.ratingKey)
      .then((colors) => {
        if (colors) usePlaybackStore.setState({ ultraBlurColors: colors });
      })
      .catch(() => {});
  }, [album, status]);

  const handleArtLoad = useCallback((e: React.SyntheticEvent<HTMLImageElement>) => {
    if (status !== "stopped") return;
    const color = extractVibrantColor(e.currentTarget);
    if (color) {
      document.documentElement.style.setProperty("--accent-r", String(color[0]));
      document.documentElement.style.setProperty("--accent-g", String(color[1]));
      document.documentElement.style.setProperty("--accent-b", String(color[2]));
    }
  }, [status]);

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
            <div className="suggestion-art-placeholder"><IconMusicNote /></div>
          )}
        </div>
        <div className="suggestion-info">
          <div className="suggestion-title">
            {album.artistName} &mdash; {album.title}{yearStr}
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
