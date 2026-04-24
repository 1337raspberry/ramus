import { useCallback, useEffect, useRef, useState } from "react";
import {
  ART_SIZE,
  search as searchCmd,
  getArtUrl,
  insertNext,
  appendToQueue,
  getTracksForAlbum,
  getTrack,
  playTracks,
  getQueue,
} from "../lib/commands";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import type { SearchResult, Track } from "../lib/types";
import { IconMusicNote, IconPlay, IconStarFilled, IconMoreDots, IconSearch } from "./Icons";
import { AlbumDownloadMenuItem, TrackDownloadMenuItem } from "./DownloadMenuItems";

interface Props {
  onDismiss: () => void;
  initialQuery?: string;
}

function SearchThumb({ artPath, onPlay }: { artPath: string | null; onPlay: () => void }) {
  const [src, setSrc] = useState<string | null>(null);
  const [err, setErr] = useState(false);

  useEffect(() => {
    if (!artPath) return;
    let cancelled = false;
    getArtUrl(artPath, ART_SIZE.SMALL)
      .then((url) => {
        if (!cancelled) setSrc(url);
      })
      .catch(() => {
        if (!cancelled) setErr(true);
      });
    return () => {
      cancelled = true;
    };
  }, [artPath]);

  return (
    <div className="search-thumb-wrap">
      {src && !err ? (
        <img className="search-thumb" src={src} alt="" onError={() => setErr(true)} />
      ) : (
        <div className="search-thumb search-thumb-placeholder">
          <IconMusicNote />
        </div>
      )}
      <button
        className="search-thumb-play"
        onClick={(e) => {
          e.stopPropagation();
          onPlay();
        }}
        title="Play"
      >
        <IconPlay />
      </button>
    </div>
  );
}

/** Fetch the full Track from the DB and run an action with it. */
async function withFullTrack(result: SearchResult, action: (track: Track) => void | Promise<void>) {
  if (!result.trackSourceId) return;
  const track = await getTrack(result.trackSourceId);
  if (track) await action(track);
}

function refreshQueue() {
  getQueue()
    .then((q) => usePlaybackStore.setState({ queue: q }))
    .catch(() => {});
}

export default function SearchOverlay({ onDismiss, initialQuery }: Props) {
  const [query, setQuery] = useState(initialQuery ?? "");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [searching, setSearching] = useState(false);
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const hasResults = results.length > 0;
  const albums = results.filter((r) => r.kind === "album");
  const tracks = results.filter((r) => r.kind === "track");
  const ordered = [...albums, ...tracks];

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!openMenuId) return;
    const handler = (e: MouseEvent) => {
      if (!(e.target as Element).closest(".search-dropdown, .search-dots-btn")) {
        setOpenMenuId(null);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [openMenuId]);

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    const trimmed = query.trim();
    if (!trimmed || /^[/@!%#]$/.test(trimmed) || trimmed.toLowerCase() === "col:") {
      setResults([]);
      setSelectedIndex(0);
      return;
    }
    setSearching(true);
    debounceRef.current = setTimeout(() => {
      searchCmd(query)
        .then((res) => {
          setResults(res);
          setSelectedIndex(0);
          setSearching(false);
        })
        .catch(() => setSearching(false));
    }, 150);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [query]);

  const handleSelect = useCallback(
    (result: SearchResult) => {
      onDismiss();
      if (result.kind === "album") {
        const store = useLibraryStore.getState();
        store.openAlbumDetail({
          ratingKey: result.albumSourceId,
          title: result.albumTitle,
          artistName: result.artistName,
          year: result.year,
          thumb: result.albumArtPath,
          genres: [],
          collections: [],
          isFavourite: result.isFavourite,
          studio: null,
          addedAt: null,
          lastViewedAt: null,
          viewCount: null,
          format: null,
          artistCountry: null,
        });
      } else {
        withFullTrack(result, (t) => playTracks([t], 0)).catch(() => {});
      }
    },
    [onDismiss],
  );

  const handlePlay = useCallback(
    (result: SearchResult) => {
      if (result.kind === "album") {
        getTracksForAlbum(result.albumSourceId)
          .then((tracks) => {
            if (tracks.length > 0) playTracks(tracks, 0);
          })
          .catch(() => {});
      } else {
        withFullTrack(result, (t) => playTracks([t], 0)).catch(() => {});
      }
      onDismiss();
    },
    [onDismiss],
  );

  const handlePlayNext = useCallback((result: SearchResult) => {
    if (result.kind === "album") {
      getTracksForAlbum(result.albumSourceId)
        .then((tracks) => insertNext(tracks))
        .then(refreshQueue)
        .catch(() => {});
    } else {
      withFullTrack(result, (t) => insertNext([t]).then(refreshQueue)).catch(() => {});
    }
    setOpenMenuId(null);
  }, []);

  const handleAddToQueue = useCallback((result: SearchResult) => {
    if (result.kind === "album") {
      getTracksForAlbum(result.albumSourceId)
        .then((tracks) => appendToQueue(tracks))
        .then(refreshQueue)
        .catch(() => {});
    } else {
      withFullTrack(result, (t) => appendToQueue([t]).then(refreshQueue)).catch(() => {});
    }
    setOpenMenuId(null);
  }, []);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onDismiss();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, ordered.length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter" && e.shiftKey) {
        e.preventDefault();
        if (query.trim()) {
          onDismiss();
          useLibraryStore.getState().loadSearchResults(query.trim());
        }
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        if (ordered[selectedIndex]) {
          handleSelect(ordered[selectedIndex]);
        }
        return;
      }
    },
    [ordered, selectedIndex, onDismiss, handleSelect, query],
  );

  const handleBackdropClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget) onDismiss();
    },
    [onDismiss],
  );

  const renderRow = (result: SearchResult, index: number) => {
    const isAlbum = result.kind === "album";
    const isMenuOpen = openMenuId === result.id;

    return (
      <div
        key={result.id}
        className={`search-row${selectedIndex === index ? " selected" : ""}`}
        onClick={() => handleSelect(result)}
        onMouseEnter={() => setSelectedIndex(index)}
      >
        <SearchThumb artPath={result.albumArtPath} onPlay={() => handlePlay(result)} />
        <div className="search-row-info">
          <div className="search-row-title">{isAlbum ? result.albumTitle : result.trackTitle}</div>
          <div className="search-row-sub">
            {isAlbum
              ? `${result.artistName}${result.year ? ` (${result.year})` : ""}`
              : `${result.trackArtist ?? result.artistName} — ${result.albumTitle}`}
          </div>
        </div>
        {result.isFavourite && (
          <span className="search-fav-star">
            <IconStarFilled />
          </span>
        )}
        <div className="search-menu-wrap">
          <button
            className="search-dots-btn"
            onClick={(e) => {
              e.stopPropagation();
              setOpenMenuId((prev) => (prev === result.id ? null : result.id));
            }}
          >
            <IconMoreDots />
          </button>
          {isMenuOpen && (
            <div className="search-dropdown">
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  handlePlayNext(result);
                }}
              >
                Play Next
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  handleAddToQueue(result);
                }}
              >
                Add to Queue
              </button>
              {result.kind === "track" && result.trackSourceId ? (
                <TrackDownloadMenuItem
                  ratingKey={result.trackSourceId}
                  onDone={() => setOpenMenuId(null)}
                />
              ) : (
                <AlbumDownloadMenuItem
                  albumRatingKey={result.albumSourceId}
                  onDone={() => setOpenMenuId(null)}
                />
              )}
            </div>
          )}
        </div>
      </div>
    );
  };

  return (
    <div className="search-backdrop" onClick={handleBackdropClick}>
      <div
        className={`search-overlay${hasResults ? " has-results" : ""}`}
        onKeyDown={handleKeyDown}
      >
        <div className="search-input-row">
          <span className="search-icon">
            <IconSearch />
          </span>
          <input
            ref={inputRef}
            className="search-input"
            type="search"
            placeholder="/genre @artist %album !track #>2000 col:name"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
          />
          {searching && <span className="search-spinner">...</span>}
        </div>

        {hasResults && (
          <div className="search-results">
            {albums.length > 0 && (
              <>
                <div className="search-section-header">Albums</div>
                {albums.map((r, i) => renderRow(r, i))}
              </>
            )}
            {tracks.length > 0 && (
              <>
                <div className="search-section-header">Tracks</div>
                {tracks.map((r, i) => renderRow(r, albums.length + i))}
              </>
            )}
          </div>
        )}

        {hasResults && albums.length > 0 && (
          <div className="search-hint">Shift+Enter to browse in grid</div>
        )}
      </div>
    </div>
  );
}
