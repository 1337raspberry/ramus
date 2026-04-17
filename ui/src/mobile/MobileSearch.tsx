import { useEffect, useRef, useState } from "react";
import {
  ART_SIZE,
  search as searchCmd,
  getArtUrl,
  getTracksForAlbum,
  getTrack,
  playTracks,
  getAlbum,
} from "../lib/commands";
import { useLibraryStore } from "../stores/libraryStore";
import type { SearchResult } from "../lib/types";
import {
  IconChevronLeft,
  IconMusicNote,
  IconClose,
  IconSearch,
  IconStarFilled,
} from "../components/Icons";

interface Props {
  onBack: () => void;
}

function SearchThumb({ path }: { path: string | null }) {
  const [src, setSrc] = useState<string | null>(null);
  const [err, setErr] = useState(false);

  useEffect(() => {
    setSrc(null);
    setErr(false);
    if (!path) return;
    let cancelled = false;
    getArtUrl(path, ART_SIZE.SMALL)
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch(() => {
        if (!cancelled) setErr(true);
      });
    return () => {
      cancelled = true;
    };
  }, [path]);

  if (src && !err) {
    return <img className="mobile-search-thumb" src={src} alt="" onError={() => setErr(true)} />;
  }
  return (
    <div className="mobile-search-thumb mobile-search-thumb-ph">
      <IconMusicNote size={18} />
    </div>
  );
}

export default function MobileSearch({ onBack }: Props) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const openAlbumDetail = useLibraryStore((s) => s.openAlbumDetail);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (!query.trim()) {
      setResults([]);
      return;
    }
    debounceRef.current = setTimeout(() => {
      searchCmd(query.trim(), 100)
        .then(setResults)
        .catch(() => setResults([]));
    }, 150);
  }, [query]);

  const albums = results.filter((r) => r.kind === "album");
  const tracks = results.filter((r) => r.kind === "track");

  const openAlbumById = async (sourceId: string) => {
    const tracksList = await getTracksForAlbum(sourceId);
    if (!tracksList.length) return;
    const first = tracksList[0];
    // Build a minimal Album shape from what we have; opening via store is
    // cleaner but we don't have the full Album object here.
    useLibraryStore.setState({ detailTracks: tracksList, tracks: tracksList });
    const album = await getAlbum(first.albumKey ?? sourceId);
    if (album) openAlbumDetail(album);
  };

  const playTrackFromResult = async (r: SearchResult) => {
    if (!r.trackSourceId) return;
    const track = await getTrack(r.trackSourceId);
    if (track) await playTracks([track], 0);
  };

  return (
    <div className="mobile-screen mobile-search">
      <header className="mobile-header">
        <button className="mobile-header-circle" onClick={onBack} aria-label="Back">
          <IconChevronLeft size={22} />
        </button>
        <div className="mobile-search-field">
          <IconSearch size={16} />
          <input
            ref={inputRef}
            className="mobile-search-input"
            type="text"
            value={query}
            placeholder="Search"
            onChange={(e) => setQuery(e.target.value)}
          />
          {query && (
            <button className="mobile-search-clear" onClick={() => setQuery("")} aria-label="Clear">
              <IconClose size={14} />
            </button>
          )}
        </div>
      </header>

      <div className="mobile-search-results">
        {albums.length > 0 && (
          <>
            <div className="mobile-search-section">Albums</div>
            {albums.map((r) => (
              <button
                key={r.id}
                className="mobile-search-row"
                onClick={() => openAlbumById(r.albumSourceId)}
              >
                <SearchThumb path={r.albumArtPath} />
                <div className="mobile-search-lines">
                  <div className="mobile-search-primary">{r.albumTitle}</div>
                  <div className="mobile-search-secondary">{r.artistName}</div>
                </div>
                {r.isFavourite && (
                  <span className="mobile-search-fav">
                    <IconStarFilled size={16} />
                  </span>
                )}
              </button>
            ))}
          </>
        )}

        {tracks.length > 0 && (
          <>
            <div className="mobile-search-section">Tracks</div>
            {tracks.map((r) => (
              <button
                key={r.id}
                className="mobile-search-row"
                onClick={() => playTrackFromResult(r)}
              >
                <SearchThumb path={r.albumArtPath} />
                <div className="mobile-search-lines">
                  <div className="mobile-search-primary">{r.trackTitle}</div>
                  <div className="mobile-search-secondary">{r.trackArtist || r.artistName}</div>
                </div>
                {r.isFavourite && (
                  <span className="mobile-search-fav">
                    <IconStarFilled size={16} />
                  </span>
                )}
              </button>
            ))}
          </>
        )}

        {query && results.length === 0 && <div className="mobile-empty">No results</div>}
      </div>
    </div>
  );
}
