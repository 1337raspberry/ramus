import { useEffect, useRef, useState } from "react";
import {
  ART_SIZE,
  search as searchCmd,
  getArtUrl,
  getTracksForAlbum,
  getTrack,
  playTracks,
  getAlbum,
  showNativeSearchBar,
  hideNativeSearchBar,
} from "../lib/commands";
import { useLibraryStore } from "../stores/libraryStore";
import type {
  SearchAlbumResult,
  SearchArtistResult,
  SearchGenreResult,
  SearchResponse,
  SearchSection,
  SearchTrackResult,
} from "../lib/types";
import { addRecent, loadRecents, removeRecent, type RecentSearch } from "../lib/searchRecents";
import {
  IconChevronLeft,
  IconChevronDown,
  IconChevronRight,
  IconMusicNote,
  IconClose,
  IconSearch,
  IconStarFilled,
  IconStarEmpty,
  IconWave,
} from "../components/Icons";

interface Props {
  onBack: () => void;
}

const IS_IOS = /iPhone|iPad|iPod/.test(navigator.userAgent);

/** Rows shown per section before "see all" expands it. */
const COLLAPSED_ROWS = 3;

const SECTION_TITLES: Record<SearchSection["kind"], string> = {
  artists: "Artists",
  albums: "Albums",
  tracks: "Tracks",
  genres: "Genres",
};

function SearchThumb({ path, round }: { path: string | null; round?: boolean }) {
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

  const cls = `mobile-search-thumb${round ? " mobile-search-thumb-round" : ""}`;
  if (src && !err) {
    return <img className={cls} src={src} alt="" onError={() => setErr(true)} />;
  }
  return (
    <div className={`${cls} mobile-search-thumb-ph`}>
      <IconMusicNote size={18} />
    </div>
  );
}

/** 5-star rating strip; `rating` is 0–10. */
function Stars({ rating }: { rating: number | null }) {
  if (rating == null || rating <= 0) return null;
  const filled = Math.round(rating / 2);
  return (
    <span className="mobile-search-stars">
      {[0, 1, 2, 3, 4].map((i) =>
        i < filled ? <IconStarFilled key={i} size={11} /> : <IconStarEmpty key={i} size={11} />,
      )}
    </span>
  );
}

function albumCountLabel(n: number) {
  return n === 1 ? "1 album" : `${n} albums`;
}

export default function MobileSearch({ onBack }: Props) {
  const [query, setQuery] = useState("");
  const [response, setResponse] = useState<SearchResponse | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [recents, setRecents] = useState<RecentSearch[]>(loadRecents);
  const openAlbumDetail = useLibraryStore((s) => s.openAlbumDetail);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const onBackRef = useRef(onBack);
  onBackRef.current = onBack;

  useEffect(() => {
    if (!IS_IOS) {
      inputRef.current?.focus();
      return;
    }
    showNativeSearchBar("");

    const onText = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      if (detail && typeof detail.text === "string") {
        setQuery(detail.text);
      }
    };
    const onCancel = () => onBackRef.current();

    window.addEventListener("nativeSearchText", onText);
    window.addEventListener("nativeSearchCancel", onCancel);

    return () => {
      hideNativeSearchBar();
      window.removeEventListener("nativeSearchText", onText);
      window.removeEventListener("nativeSearchCancel", onCancel);
    };
  }, []);

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (!query.trim()) {
      setResponse(null);
      setExpanded(new Set());
      return;
    }
    debounceRef.current = setTimeout(() => {
      searchCmd(query.trim(), 30)
        .then((res) => {
          setResponse(res);
          setExpanded(new Set());
        })
        .catch(() => setResponse(null));
    }, 150);
  }, [query]);

  const openAlbumById = async (sourceId: string) => {
    const tracksList = await getTracksForAlbum(sourceId);
    if (!tracksList.length) return;
    const first = tracksList[0];
    useLibraryStore.setState({ detailTracks: tracksList, tracks: tracksList });
    const album = await getAlbum(first.albumKey ?? sourceId);
    if (album) openAlbumDetail(album);
  };

  const playTrackById = async (sourceId: string) => {
    const track = await getTrack(sourceId);
    if (track) await playTracks([track], 0);
  };

  const openArtist = (name: string) => {
    // Clears searchQuery itself, which flips MobileApp out of the search
    // view and into the artist's album grid.
    void useLibraryStore.getState().loadAlbumsForArtistName(name);
  };

  const openGenre = (name: string) => {
    void useLibraryStore.getState().selectGenreByName(name);
  };

  const tapArtist = (r: SearchArtistResult) => {
    setRecents(addRecent({ kind: "artist", id: r.sourceId, name: r.name, artUrl: r.artUrl }));
    openArtist(r.name);
  };

  const tapAlbum = (r: SearchAlbumResult) => {
    setRecents(
      addRecent({
        kind: "album",
        id: r.sourceId,
        title: r.title,
        artistName: r.artistName,
        artUrl: r.artUrl,
      }),
    );
    void openAlbumById(r.sourceId);
  };

  const tapTrack = (r: SearchTrackResult) => {
    setRecents(
      addRecent({
        kind: "track",
        id: r.sourceId,
        title: r.title,
        artist: r.displayArtist,
        artUrl: r.artUrl,
      }),
    );
    void playTrackById(r.sourceId);
  };

  const tapGenre = (r: SearchGenreResult) => {
    setRecents(addRecent({ kind: "genre", id: r.name, name: r.name }));
    openGenre(r.name);
  };

  const renderArtistRow = (r: SearchArtistResult) => (
    <button key={`ar-${r.sourceId}`} className="mobile-search-row" onClick={() => tapArtist(r)}>
      <SearchThumb path={r.artUrl} round />
      <div className="mobile-search-lines">
        <div className="mobile-search-primary">{r.name}</div>
        <div className="mobile-search-secondary">{albumCountLabel(r.albumCount)}</div>
      </div>
    </button>
  );

  const renderAlbumRow = (r: SearchAlbumResult) => (
    <button key={`al-${r.sourceId}`} className="mobile-search-row" onClick={() => tapAlbum(r)}>
      <SearchThumb path={r.artUrl} />
      <div className="mobile-search-lines">
        <div className="mobile-search-primary">{r.title}</div>
        <div className="mobile-search-secondary mobile-search-subrow">
          <span className="mobile-search-subtext">{r.artistName}</span>
          <Stars rating={r.rating} />
          {r.quality && <span className="mobile-search-quality">{r.quality}</span>}
        </div>
      </div>
      {r.isFavourite && (
        <span className="mobile-search-fav">
          <IconStarFilled size={16} />
        </span>
      )}
    </button>
  );

  const renderTrackRow = (r: SearchTrackResult) => (
    <button key={`tr-${r.sourceId}`} className="mobile-search-row" onClick={() => tapTrack(r)}>
      <SearchThumb path={r.artUrl} />
      <div className="mobile-search-lines">
        <div className="mobile-search-primary">{r.title}</div>
        <div className="mobile-search-secondary mobile-search-subrow">
          <span className="mobile-search-subtext">{r.displayArtist}</span>
          <Stars rating={r.rating} />
        </div>
      </div>
      {r.isFavourite && (
        <span className="mobile-search-fav">
          <IconStarFilled size={16} />
        </span>
      )}
    </button>
  );

  const renderGenreRow = (r: SearchGenreResult) => (
    <button key={`ge-${r.name}`} className="mobile-search-row" onClick={() => tapGenre(r)}>
      <div className="mobile-search-thumb mobile-search-thumb-ph mobile-search-genre-icon">
        <IconWave size={20} />
      </div>
      <div className="mobile-search-lines">
        <div className="mobile-search-primary">{r.name}</div>
        <div className="mobile-search-secondary">{albumCountLabel(r.albumCount)}</div>
      </div>
    </button>
  );

  const renderSection = (section: SearchSection) => {
    const isExpanded = expanded.has(section.kind);
    const hasMore = section.items.length > COLLAPSED_ROWS;
    const visible = isExpanded ? section.items : section.items.slice(0, COLLAPSED_ROWS);

    const toggle = () => {
      if (!hasMore) return;
      setExpanded((prev) => {
        const next = new Set(prev);
        if (next.has(section.kind)) next.delete(section.kind);
        else next.add(section.kind);
        return next;
      });
    };

    return (
      <div key={section.kind}>
        <button
          className={`mobile-search-section${hasMore ? " has-more" : ""}`}
          onClick={toggle}
          disabled={!hasMore}
        >
          <span>{SECTION_TITLES[section.kind]}</span>
          {hasMore && (isExpanded ? <IconChevronDown size={18} /> : <IconChevronRight size={18} />)}
        </button>
        {section.kind === "artists" && (visible as SearchArtistResult[]).map(renderArtistRow)}
        {section.kind === "albums" && (visible as SearchAlbumResult[]).map(renderAlbumRow)}
        {section.kind === "tracks" && (visible as SearchTrackResult[]).map(renderTrackRow)}
        {section.kind === "genres" && (visible as SearchGenreResult[]).map(renderGenreRow)}
      </div>
    );
  };

  const tapRecent = (r: RecentSearch) => {
    setRecents(addRecent(r));
    switch (r.kind) {
      case "artist":
        openArtist(r.name);
        break;
      case "album":
        void openAlbumById(r.id);
        break;
      case "track":
        void playTrackById(r.id);
        break;
      case "genre":
        openGenre(r.name);
        break;
    }
  };

  const renderRecentRow = (r: RecentSearch) => (
    <div key={`rc-${r.kind}-${r.id}`} className="mobile-search-row mobile-search-recent-row">
      <button className="mobile-search-recent-main" onClick={() => tapRecent(r)}>
        {r.kind === "genre" ? (
          <div className="mobile-search-thumb mobile-search-thumb-ph mobile-search-genre-icon">
            <IconWave size={20} />
          </div>
        ) : (
          <SearchThumb path={r.artUrl} round={r.kind === "artist"} />
        )}
        <div className="mobile-search-lines">
          <div className="mobile-search-primary">
            {r.kind === "album" || r.kind === "track" ? r.title : r.name}
          </div>
          <div className="mobile-search-secondary">
            {r.kind === "album" && r.artistName}
            {r.kind === "track" && r.artist}
            {r.kind === "artist" && "Artist"}
            {r.kind === "genre" && "Genre"}
          </div>
        </div>
      </button>
      <button
        className="mobile-search-recent-x"
        onClick={() => setRecents(removeRecent(r.kind, r.id))}
        aria-label="Remove from recent searches"
      >
        <IconClose size={14} />
      </button>
    </div>
  );

  const trimmed = query.trim();
  const sections = response?.sections ?? [];

  return (
    <div className="mobile-screen mobile-search">
      {IS_IOS ? (
        <div style={{ height: 56, flexShrink: 0 }} />
      ) : (
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
              placeholder="Search..."
              onChange={(e) => setQuery(e.target.value)}
            />
            {query && (
              <button
                className="mobile-search-clear"
                onClick={() => setQuery("")}
                aria-label="Clear"
              >
                <IconClose size={14} />
              </button>
            )}
          </div>
        </header>
      )}

      <div className="mobile-search-results">
        {!trimmed && recents.length > 0 && (
          <>
            <div className="mobile-search-section">
              <span>Recent searches</span>
            </div>
            {recents.map(renderRecentRow)}
          </>
        )}

        {trimmed && sections.map(renderSection)}

        {trimmed && response && sections.length === 0 && (
          <div className="mobile-empty">No matches in your library</div>
        )}
      </div>
    </div>
  );
}
