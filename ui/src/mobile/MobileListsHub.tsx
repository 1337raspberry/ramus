import { useEffect, useMemo, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import { ART_SIZE, getAllCollectionNames, getPlaylists } from "../lib/commands";
import { pushBackHandler } from "../lib/backHandler";
import { describeFilters } from "../lib/filterDescribe";
import { filtersFromBookmark } from "../lib/bookmark";
import { useArtUrl } from "../lib/useArtUrl";
import { IconChevronRight, IconMusicNote } from "../components/Icons";
import BookmarkEditor from "../components/BookmarkEditor";
import SmartPlaylistBuilder from "./SmartPlaylistBuilder";
import type { Bookmark, Playlist } from "../lib/types";

/** Small square thumb for hub rows — playlist composites and collection
 * stand-ins share the one placeholder look. */
function HubThumb({ thumb }: { thumb: string | null }) {
  const { artSrc, artErr, setArtErr } = useArtUrl(thumb, ART_SIZE.SMALL);
  return (
    <div className="mobile-lists-thumb">
      {artSrc && !artErr ? (
        <img src={artSrc} alt="" onError={() => setArtErr(true)} />
      ) : (
        <div className="mobile-lists-thumb-ph">
          <IconMusicNote size={14} />
        </div>
      )}
    </div>
  );
}

interface Props {
  /** Switch the app back to the grid view after a Smart Filter is applied. */
  onOpenGrid: () => void;
}

/**
 * Top-level "Lists" view: every named grouping in one place — Collections
 * (browse surface) and Smart Filters (saved filter snapshots, formerly
 * "bookmarks"). Playlists join this hub when they land.
 */
export default function MobileListsHub({ onOpenGrid }: Props) {
  const bookmarks = useSettingsStore((s) => s.bookmarks);
  const loadAlbumsForCollection = useLibraryStore((s) => s.loadAlbumsForCollection);
  const albums = useLibraryStore((s) => s.unfilteredAlbums);
  const [collections, setCollections] = useState<string[]>([]);
  const [playlists, setPlaylists] = useState<Playlist[] | null>(null);
  const [showEditor, setShowEditor] = useState(false);
  const [showSmartBuilder, setShowSmartBuilder] = useState(false);

  useEffect(() => {
    getAllCollectionNames()
      .then(setCollections)
      .catch(() => {});
    getPlaylists()
      .then(setPlaylists)
      .catch(() => setPlaylists([]));
  }, []);

  useEffect(() => {
    if (!showEditor) return;
    return pushBackHandler(() => {
      setShowEditor(false);
      return true;
    });
  }, [showEditor]);

  // Album counts per collection, derived from whatever album list is already
  // in the store (cheap and usually the full library). Purely cosmetic — a
  // missing count renders as nothing rather than 0.
  const collectionCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const a of albums) {
      for (const c of a.collections) {
        const key = c.toLowerCase();
        counts.set(key, (counts.get(key) ?? 0) + 1);
      }
    }
    return counts;
  }, [albums]);

  // First member album's art stands in for each collection — the mirror
  // stores collections as name tags off the albums, not as server objects,
  // so Plex's own collection composite isn't on hand. Same best-effort
  // source (and caveat) as the counts above.
  const collectionArt = useMemo(() => {
    const art = new Map<string, string>();
    for (const a of albums) {
      if (!a.thumb) continue;
      for (const c of a.collections) {
        const key = c.toLowerCase();
        if (!art.has(key)) art.set(key, a.thumb);
      }
    }
    return art;
  }, [albums]);

  const summaries = useMemo(
    () => bookmarks.map((b) => describeFilters(filtersFromBookmark(b))),
    [bookmarks],
  );

  const applyBookmark = (entry: Bookmark) => {
    useLibraryStore.setState({
      detailAlbum: null,
      detailTracks: [],
      suggestion: null,
      suggestionMissed: false,
      searchQuery: null,
      browseArtistName: null,
      browseYear: null,
      browseCollectionName: null,
      browsePlaylist: null,
      selectedGenreId: "__all__",
      selectedArtistId: null,
    });
    usePlaybackStore.setState({ isFocusMode: false });
    useLibraryStore.getState().loadBookmark(filtersFromBookmark(entry), entry.name);
    onOpenGrid();
  };

  return (
    <div className="mobile-lists-hub">
      <div className="mobile-lists-section-title">Playlists</div>
      {playlists === null ? (
        <div className="mobile-lists-empty">Loading…</div>
      ) : playlists.length === 0 ? (
        <div className="mobile-lists-empty">
          No playlists yet. Save the queue as one from the player&rsquo;s … menu.
        </div>
      ) : (
        playlists.map((p) => (
          <button
            key={p.sourceId}
            className="mobile-artist-row"
            onClick={() => useLibraryStore.setState({ browsePlaylist: p })}
          >
            <HubThumb thumb={p.thumb} />
            <span className="mobile-lists-row-name">{p.title}</span>
            {p.smart && <span className="mobile-lists-smart-badge">SMART</span>}
            {p.trackCount != null && <span className="mobile-lists-row-count">{p.trackCount}</span>}
            <IconChevronRight size={18} className="mobile-lists-chevron" />
          </button>
        ))
      )}
      <button className="mobile-lists-manage" onClick={() => setShowSmartBuilder(true)}>
        New Smart Playlist…
      </button>

      <div className="mobile-lists-section-title">Collections</div>
      {collections.length === 0 ? (
        <div className="mobile-lists-empty">
          No collections yet. Add an album to one from its … menu.
        </div>
      ) : (
        collections.map((name) => {
          const count = collectionCounts.get(name.toLowerCase());
          return (
            <button
              key={name}
              className="mobile-artist-row"
              onClick={() => loadAlbumsForCollection(name)}
            >
              <HubThumb thumb={collectionArt.get(name.toLowerCase()) ?? null} />
              <span className="mobile-lists-row-name">{name}</span>
              {count != null && <span className="mobile-lists-row-count">{count}</span>}
              <IconChevronRight size={18} className="mobile-lists-chevron" />
            </button>
          );
        })
      )}

      <div className="mobile-lists-section-title">Smart Filters</div>
      {bookmarks.length === 0 ? (
        <div className="mobile-lists-empty">
          No Smart Filters yet. Set a filter, then save it from the filter panel&rsquo;s … menu.
        </div>
      ) : (
        bookmarks.map((entry, i) => (
          <button key={entry.id} className="mobile-artist-row" onClick={() => applyBookmark(entry)}>
            <span className="mobile-lists-row-name">{entry.name}</span>
            <span className="mobile-lists-row-summary">{summaries[i]}</span>
            <IconChevronRight size={18} className="mobile-lists-chevron" />
          </button>
        ))
      )}
      {bookmarks.length > 0 && (
        <button className="mobile-lists-manage" onClick={() => setShowEditor(true)}>
          Manage Smart Filters…
        </button>
      )}

      {showEditor && <BookmarkEditor onDismiss={() => setShowEditor(false)} />}
      {showSmartBuilder && (
        <SmartPlaylistBuilder
          onDismiss={() => setShowSmartBuilder(false)}
          onCreated={(p) => {
            setShowSmartBuilder(false);
            // Refresh the hub list for back-nav, and open the new playlist
            // so its computed tracks show immediately.
            getPlaylists()
              .then(setPlaylists)
              .catch(() => {});
            useLibraryStore.setState({ browsePlaylist: p });
          }}
        />
      )}
    </div>
  );
}
