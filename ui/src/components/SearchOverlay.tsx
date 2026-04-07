import { useCallback, useEffect, useRef, useState } from "react";
import { search as searchCmd, getArtUrl, insertNext, appendToQueue, getTracksForAlbum, playTracks } from "../lib/commands";
import type { SearchResult, Track } from "../lib/types";
import { useLibraryStore } from "../stores/libraryStore";

interface Props {
  onDismiss: () => void;
}

function SearchThumb({ artPath }: { artPath: string | null }) {
  const [src, setSrc] = useState<string | null>(null);
  const [err, setErr] = useState(false);

  if (artPath && !src && !err) {
    getArtUrl(artPath, 72)
      .then(setSrc)
      .catch(() => setErr(true));
  }

  if (src && !err) {
    return <img className="search-thumb" src={src} alt="" onError={() => setErr(true)} />;
  }
  return <div className="search-thumb search-thumb-placeholder">{"\u266B"}</div>;
}

export default function SearchOverlay({ onDismiss }: Props) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [searching, setSearching] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const hasResults = results.length > 0;
  const albums = results.filter((r) => r.kind === "album");
  const tracks = results.filter((r) => r.kind === "track");
  // Ordered list for keyboard nav: albums first, then tracks
  const ordered = [...albums, ...tracks];

  // Focus input on mount
  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Debounced search
  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (!query.trim()) {
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
        // Load album tracks and play from the start
        getTracksForAlbum(result.albumSourceId)
          .then((tracks) => {
            if (tracks.length > 0) playTracks(tracks, 0);
          })
          .catch(() => {});
      } else if (result.trackSourceId) {
        // Play the individual track
        const track: Track = {
          ratingKey: result.trackSourceId,
          title: result.trackTitle ?? result.albumTitle,
          artistName: result.trackArtist ?? result.artistName,
          trackArtist: result.trackArtist,
          albumTitle: result.albumTitle,
          albumKey: result.albumSourceId,
          index: null,
          duration: 0,
          codec: null,
          partKey: null,
          thumb: result.albumArtPath,
          isFavourite: false,
          bitrate: null,
          discNumber: null,
        };
        playTracks([track], 0).catch(() => {});
      }
    },
    [onDismiss]
  );

  const handlePlayNext = useCallback((result: SearchResult) => {
    // Build a minimal track to insert
    if (result.trackSourceId) {
      const track: Track = {
        ratingKey: result.trackSourceId,
        title: result.trackTitle ?? result.albumTitle,
        artistName: result.trackArtist ?? result.artistName,
        trackArtist: result.trackArtist,
        albumTitle: result.albumTitle,
        albumKey: result.albumSourceId,
        index: null,
        duration: 0,
        codec: null,
        partKey: null,
        thumb: result.albumArtPath,
        isFavourite: false,
        bitrate: null,
        discNumber: null,
      };
      insertNext([track]).catch(() => {});
    }
  }, []);

  const handleAddToQueue = useCallback((result: SearchResult) => {
    if (result.trackSourceId) {
      const track: Track = {
        ratingKey: result.trackSourceId,
        title: result.trackTitle ?? result.albumTitle,
        artistName: result.trackArtist ?? result.artistName,
        trackArtist: result.trackArtist,
        albumTitle: result.albumTitle,
        albumKey: result.albumSourceId,
        index: null,
        duration: 0,
        codec: null,
        partKey: null,
        thumb: result.albumArtPath,
        isFavourite: false,
        bitrate: null,
        discNumber: null,
      };
      appendToQueue([track]).catch(() => {});
    }
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
      if (e.key === "Enter") {
        e.preventDefault();
        if (e.shiftKey) {
          // Load all album results into grid
          if (albums.length > 0) {
            onDismiss();
          }
        } else if (ordered[selectedIndex]) {
          handleSelect(ordered[selectedIndex]);
        }
        return;
      }
    },
    [ordered, selectedIndex, albums, onDismiss, handleSelect]
  );

  // Click backdrop to dismiss
  const handleBackdropClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget) onDismiss();
    },
    [onDismiss]
  );

  return (
    <div className="search-backdrop" onClick={handleBackdropClick}>
      <div
        className={`search-overlay${hasResults ? " has-results" : ""}`}
        onKeyDown={handleKeyDown}
      >
        <div className="search-input-row">
          <span className="search-icon">{"\uD83D\uDD0D"}</span>
          <input
            ref={inputRef}
            className="search-input"
            type="text"
            placeholder="/genre @artist %album !track year:>2000"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          {searching && <span className="search-spinner">...</span>}
        </div>

        {hasResults && (
          <div className="search-results">
            {albums.length > 0 && (
              <>
                <div className="search-section-header">Albums</div>
                {albums.map((result, i) => (
                  <div
                    key={result.id}
                    className={`search-row${selectedIndex === i ? " selected" : ""}`}
                    onClick={() => handleSelect(result)}
                    onMouseEnter={() => setSelectedIndex(i)}
                  >
                    <SearchThumb artPath={result.albumArtPath} />
                    <div className="search-row-info">
                      <div className="search-row-title">{result.albumTitle}</div>
                      <div className="search-row-sub">
                        {result.artistName}
                        {result.year ? ` (${result.year})` : ""}
                      </div>
                    </div>
                    <button
                      className="search-action-btn"
                      title="Play Next"
                      onClick={(e) => {
                        e.stopPropagation();
                        handlePlayNext(result);
                      }}
                    >
                      {"\u25B6"}
                    </button>
                    <button
                      className="search-action-btn"
                      title="Add to Queue"
                      onClick={(e) => {
                        e.stopPropagation();
                        handleAddToQueue(result);
                      }}
                    >
                      +
                    </button>
                  </div>
                ))}
              </>
            )}

            {tracks.length > 0 && (
              <>
                <div className="search-section-header">Tracks</div>
                {tracks.map((result, i) => {
                  const globalIndex = albums.length + i;
                  return (
                    <div
                      key={result.id}
                      className={`search-row${selectedIndex === globalIndex ? " selected" : ""}`}
                      onClick={() => handleSelect(result)}
                      onMouseEnter={() => setSelectedIndex(globalIndex)}
                    >
                      <SearchThumb artPath={result.albumArtPath} />
                      <div className="search-row-info">
                        <div className="search-row-title">{result.trackTitle}</div>
                        <div className="search-row-sub">
                          {result.trackArtist ?? result.artistName} — {result.albumTitle}
                        </div>
                      </div>
                      <button
                        className="search-action-btn"
                        title="Play Next"
                        onClick={(e) => {
                          e.stopPropagation();
                          handlePlayNext(result);
                        }}
                      >
                        {"\u25B6"}
                      </button>
                      <button
                        className="search-action-btn"
                        title="Add to Queue"
                        onClick={(e) => {
                          e.stopPropagation();
                          handleAddToQueue(result);
                        }}
                      >
                        +
                      </button>
                    </div>
                  );
                })}
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
