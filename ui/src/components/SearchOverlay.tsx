import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ART_SIZE,
  search as searchCmd,
  insertNext,
  appendToQueue,
  getTracksForAlbum,
  getTrack,
  playTracks,
  getQueue,
} from "../lib/commands";
import { useArtUrl } from "../lib/useArtUrl";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import type {
  SearchAlbumResult,
  SearchArtistResult,
  SearchGenreResult,
  SearchResponse,
  SearchSection,
  SearchTrackResult,
  Track,
} from "../lib/types";
import {
  IconMusicNote,
  IconPlay,
  IconStarFilled,
  IconMoreDots,
  IconSearch,
  IconWave,
} from "./Icons";
import { AlbumDownloadMenuItem, TrackDownloadMenuItem } from "./DownloadMenuItems";

interface Props {
  onDismiss: () => void;
  initialQuery?: string;
}

const SECTION_TITLES: Record<SearchSection["kind"], string> = {
  artists: "Artists",
  albums: "Albums",
  tracks: "Tracks",
  genres: "Genres",
};

/** One selectable row, flattened across sections for keyboard nav. */
type FlatRow =
  | { kind: "artist"; id: string; item: SearchArtistResult }
  | { kind: "album"; id: string; item: SearchAlbumResult }
  | { kind: "track"; id: string; item: SearchTrackResult }
  | { kind: "genre"; id: string; item: SearchGenreResult };

function flattenSections(sections: SearchSection[]): FlatRow[] {
  const rows: FlatRow[] = [];
  for (const section of sections) {
    switch (section.kind) {
      case "artists":
        for (const item of section.items)
          rows.push({ kind: "artist", id: `ar-${item.sourceId}`, item });
        break;
      case "albums":
        for (const item of section.items)
          rows.push({ kind: "album", id: `al-${item.sourceId}`, item });
        break;
      case "tracks":
        for (const item of section.items)
          rows.push({ kind: "track", id: `tr-${item.sourceId}`, item });
        break;
      case "genres":
        for (const item of section.items) rows.push({ kind: "genre", id: `ge-${item.name}`, item });
        break;
    }
  }
  return rows;
}

function SearchThumb({
  artPath,
  round,
  onPlay,
}: {
  artPath: string | null;
  round?: boolean;
  onPlay?: () => void;
}) {
  const { artSrc: src, artErr: err, setArtErr: setErr } = useArtUrl(artPath, ART_SIZE.SMALL);

  return (
    <div className={`search-thumb-wrap${round ? " search-thumb-round" : ""}`}>
      {src && !err ? (
        <img className="search-thumb" src={src} alt="" onError={() => setErr(true)} />
      ) : (
        <div className="search-thumb search-thumb-placeholder">
          <IconMusicNote />
        </div>
      )}
      {onPlay && (
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
      )}
    </div>
  );
}

/** Fetch the full Track from the DB and run an action with it. */
async function withFullTrack(sourceId: string, action: (track: Track) => void | Promise<void>) {
  const track = await getTrack(sourceId);
  if (track) await action(track);
}

function refreshQueue() {
  getQueue()
    .then((q) => usePlaybackStore.setState({ queue: q }))
    .catch(() => {});
}

export default function SearchOverlay({ onDismiss, initialQuery }: Props) {
  const [query, setQuery] = useState(initialQuery ?? "");
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [searching, setSearching] = useState(false);
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const sections = useMemo(() => response?.sections ?? [], [response]);
  const flat = useMemo(() => flattenSections(sections), [sections]);
  const hasResults = flat.length > 0;
  const hasAlbums = sections.some((s) => s.kind === "albums");

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
      setResponse(null);
      setSelectedIndex(0);
      // Reset the spinner — clearing the input cancels the pending
      // timeout that would otherwise have flipped it back to false,
      // leaving it stuck on after a fast type-then-clear sequence.
      setSearching(false);
      return;
    }
    setSearching(true);
    debounceRef.current = setTimeout(() => {
      searchCmd(query)
        .then((res) => {
          setResponse(res);
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
    (row: FlatRow) => {
      onDismiss();
      const store = useLibraryStore.getState();
      switch (row.kind) {
        case "artist":
          void store.loadAlbumsForArtistName(row.item.name);
          break;
        case "album":
          store.openAlbumDetail({
            ratingKey: row.item.sourceId,
            title: row.item.title,
            artistName: row.item.artistName,
            year: row.item.year,
            thumb: row.item.artUrl,
            genres: [],
            collections: [],
            isFavourite: row.item.isFavourite,
            hasFavouriteTrack: false,
            studio: null,
            addedAt: null,
            lastViewedAt: null,
            viewCount: null,
            format: null,
            artistCountry: null,
          });
          break;
        case "track":
          withFullTrack(row.item.sourceId, (t) => playTracks([t], 0)).catch(() => {});
          break;
        case "genre":
          void store.selectGenreByName(row.item.name);
          break;
      }
    },
    [onDismiss],
  );

  const handlePlayAlbum = useCallback(
    (item: SearchAlbumResult) => {
      getTracksForAlbum(item.sourceId)
        .then((tracks) => {
          if (tracks.length > 0) playTracks(tracks, 0);
        })
        .catch(() => {});
      onDismiss();
    },
    [onDismiss],
  );

  const handlePlayTrack = useCallback(
    (item: SearchTrackResult) => {
      withFullTrack(item.sourceId, (t) => playTracks([t], 0)).catch(() => {});
      onDismiss();
    },
    [onDismiss],
  );

  const handlePlayNext = useCallback((row: FlatRow) => {
    if (row.kind === "album") {
      getTracksForAlbum(row.item.sourceId)
        .then((tracks) => insertNext(tracks))
        .then(refreshQueue)
        .catch(() => {});
    } else if (row.kind === "track") {
      withFullTrack(row.item.sourceId, (t) => insertNext([t]).then(refreshQueue)).catch(() => {});
    }
    setOpenMenuId(null);
  }, []);

  const handleAddToQueue = useCallback((row: FlatRow) => {
    if (row.kind === "album") {
      getTracksForAlbum(row.item.sourceId)
        .then((tracks) => appendToQueue(tracks))
        .then(refreshQueue)
        .catch(() => {});
    } else if (row.kind === "track") {
      withFullTrack(row.item.sourceId, (t) => appendToQueue([t]).then(refreshQueue)).catch(
        () => {},
      );
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
        setSelectedIndex((i) => Math.min(i + 1, flat.length - 1));
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
        if (flat[selectedIndex]) {
          handleSelect(flat[selectedIndex]);
        }
        return;
      }
    },
    [flat, selectedIndex, onDismiss, handleSelect, query],
  );

  const handleBackdropClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget) onDismiss();
    },
    [onDismiss],
  );

  const renderMenu = (row: FlatRow) => {
    if (row.kind !== "album" && row.kind !== "track") return null;
    const isMenuOpen = openMenuId === row.id;
    return (
      <div className="search-menu-wrap">
        <button
          className="search-dots-btn"
          onClick={(e) => {
            e.stopPropagation();
            setOpenMenuId((prev) => (prev === row.id ? null : row.id));
          }}
        >
          <IconMoreDots />
        </button>
        {isMenuOpen && (
          <div className="search-dropdown">
            <button
              onClick={(e) => {
                e.stopPropagation();
                handlePlayNext(row);
              }}
            >
              Play Next
            </button>
            <button
              onClick={(e) => {
                e.stopPropagation();
                handleAddToQueue(row);
              }}
            >
              Add to Queue
            </button>
            {row.kind === "track" ? (
              <TrackDownloadMenuItem
                ratingKey={row.item.sourceId}
                onDone={() => setOpenMenuId(null)}
              />
            ) : (
              <AlbumDownloadMenuItem
                albumRatingKey={row.item.sourceId}
                onDone={() => setOpenMenuId(null)}
              />
            )}
          </div>
        )}
      </div>
    );
  };

  const renderRow = (row: FlatRow, index: number) => {
    let thumb: React.ReactNode;
    let title: string;
    let sub: React.ReactNode;
    let fav = false;

    switch (row.kind) {
      case "artist":
        thumb = <SearchThumb artPath={row.item.artUrl} round />;
        title = row.item.name;
        sub = row.item.albumCount === 1 ? "1 album" : `${row.item.albumCount} albums`;
        break;
      case "album":
        thumb = <SearchThumb artPath={row.item.artUrl} onPlay={() => handlePlayAlbum(row.item)} />;
        title = row.item.title;
        sub = (
          <>
            {row.item.artistName}
            {row.item.year ? ` (${row.item.year})` : ""}
            {row.item.quality && <span className="search-quality">{row.item.quality}</span>}
          </>
        );
        fav = row.item.isFavourite;
        break;
      case "track":
        thumb = <SearchThumb artPath={row.item.artUrl} onPlay={() => handlePlayTrack(row.item)} />;
        title = row.item.title;
        sub = `${row.item.displayArtist} — ${row.item.albumTitle}`;
        fav = row.item.isFavourite;
        break;
      case "genre":
        thumb = (
          <div className="search-thumb-wrap">
            <div className="search-thumb search-thumb-placeholder search-genre-icon">
              <IconWave />
            </div>
          </div>
        );
        title = row.item.name;
        sub = row.item.albumCount === 1 ? "1 album" : `${row.item.albumCount} albums`;
        break;
    }

    return (
      <div
        key={row.id}
        className={`search-row${selectedIndex === index ? " selected" : ""}`}
        onClick={() => handleSelect(row)}
        onMouseEnter={() => setSelectedIndex(index)}
      >
        {thumb}
        <div className="search-row-info">
          <div className="search-row-title">{title}</div>
          <div className="search-row-sub">{sub}</div>
        </div>
        {fav && (
          <span className="search-fav-star">
            <IconStarFilled />
          </span>
        )}
        {renderMenu(row)}
      </div>
    );
  };

  // Rows render grouped by section, but keyboard selection runs over the
  // flattened list — track the running index across sections.
  let runningIndex = 0;

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
            {sections.map((section) => {
              const rows = flat.slice(runningIndex, runningIndex + section.items.length);
              const start = runningIndex;
              runningIndex += section.items.length;
              return (
                <div key={section.kind}>
                  <div className="search-section-header">{SECTION_TITLES[section.kind]}</div>
                  {rows.map((row, i) => renderRow(row, start + i))}
                </div>
              );
            })}
          </div>
        )}

        {hasResults && hasAlbums && (
          <div className="search-hint">Shift+Enter to browse in grid</div>
        )}
      </div>
    </div>
  );
}
