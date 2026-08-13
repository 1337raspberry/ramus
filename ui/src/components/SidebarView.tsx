import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useLibraryStore, type SidebarMode } from "../stores/libraryStore";
import { useSettingsStore } from "../stores/settingsStore";
import GenreTreeView from "./GenreTreeView";
import BookmarkEditor from "./BookmarkEditor";
import { filtersFromBookmark } from "../lib/bookmark";
import { describeFilters } from "../lib/filterDescribe";
import { getAllCollectionNames, getPlaylists } from "../lib/commands";
import { countryToFlag } from "../lib/countryFlag";
import type { Bookmark, Playlist } from "../lib/types";

const TEXT_SIZE = 12;
const PAD_H = 6;
const ROW_HEIGHT = 30;
const CHEVRON_WIDTH = 20;

const TABS: { mode: SidebarMode; label: string }[] = [
  { mode: "genres", label: "Genres" },
  { mode: "artists", label: "Artists" },
  { mode: "lists", label: "Lists" },
];

interface SidebarProps {
  onOpenSettings?: () => void;
}

function ArtistList({
  artists,
  selectedArtistId,
  selectArtist,
}: {
  artists: { sourceId: string; name: string; country?: string | null }[];
  selectedArtistId: string | null;
  selectArtist: (id: string) => void;
}) {
  const libraryPadding = useSettingsStore((s) => s.libraryPadding);
  const showArtistFlags = useSettingsStore((s) => s.showArtistFlags);
  const effectiveRowHeight = Math.max(12, ROW_HEIGHT + libraryPadding * 2);
  const parentRef = useRef<HTMLDivElement>(null);

  const estimateSize = useCallback(() => effectiveRowHeight, [effectiveRowHeight]);
  const virtualizer = useVirtualizer({
    count: artists.length,
    getScrollElement: () => parentRef.current,
    estimateSize,
    overscan: 20,
  });

  useEffect(() => {
    virtualizer.measure();
  }, [effectiveRowHeight, virtualizer]);

  if (artists.length === 0) {
    return <div className="empty-state">No artists loaded</div>;
  }

  return (
    <div ref={parentRef} style={{ height: "100%", overflow: "auto", paddingTop: 2 }}>
      <div
        style={{
          height: virtualizer.getTotalSize(),
          width: "100%",
          position: "relative",
        }}
      >
        {virtualizer.getVirtualItems().map((vItem) => {
          const artist = artists[vItem.index];
          return (
            <div
              key={artist.sourceId}
              className={`genre-row${selectedArtistId === artist.sourceId ? " selected" : ""}`}
              style={{
                position: "absolute",
                top: vItem.start,
                left: 0,
                right: 0,
                height: effectiveRowHeight,
                display: "flex",
                alignItems: "center",
                paddingLeft: PAD_H,
                paddingRight: PAD_H,
                fontSize: TEXT_SIZE,
                cursor: "pointer",
                whiteSpace: "nowrap",
                overflow: "hidden",
              }}
              onClick={() => selectArtist(artist.sourceId)}
            >
              <span
                style={{
                  width: CHEVRON_WIDTH,
                  flexShrink: 0,
                  textAlign: "center",
                  fontSize: TEXT_SIZE,
                }}
              >
                {showArtistFlags && artist.country ? (countryToFlag(artist.country) ?? "") : ""}
              </span>
              <span className="genre-name">{artist.name}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

/**
 * "Lists" sidebar tab: Playlists, Collections (browse) and Smart Filters
 * (saved filter snapshots) in one place.
 */
function ListsPanel({
  onLoadBookmark,
  onManage,
}: {
  onLoadBookmark: (entry: Bookmark) => void;
  onManage: () => void;
}) {
  const bookmarks = useSettingsStore((s) => s.bookmarks);
  const loadAlbumsForCollection = useLibraryStore((s) => s.loadAlbumsForCollection);
  const browseCollectionName = useLibraryStore((s) => s.browseCollectionName);
  const browsePlaylist = useLibraryStore((s) => s.browsePlaylist);
  const activeBookmarkName = useLibraryStore((s) => s.activeBookmarkName);
  const [collections, setCollections] = useState<string[]>([]);
  const [playlists, setPlaylists] = useState<Playlist[] | null>(null);

  useEffect(() => {
    getAllCollectionNames()
      .then(setCollections)
      .catch(() => {});
    getPlaylists()
      .then(setPlaylists)
      .catch(() => setPlaylists([]));
  }, []);

  const summaries = useMemo(
    () => bookmarks.map((b) => describeFilters(filtersFromBookmark(b))),
    [bookmarks],
  );

  return (
    <div className="lists-panel">
      <div className="lists-panel-heading">Playlists</div>
      {playlists === null ? (
        <div className="lists-panel-empty">Loading…</div>
      ) : playlists.length === 0 ? (
        <div className="lists-panel-empty">
          No playlists yet. Save the queue as one from the player&rsquo;s … menu.
        </div>
      ) : (
        playlists.map((p) => (
          <button
            key={p.sourceId}
            className={`lists-panel-row${browsePlaylist?.sourceId === p.sourceId ? " selected" : ""}`}
            onClick={() => useLibraryStore.setState({ browsePlaylist: p, detailAlbum: null })}
          >
            <span className="lists-panel-row-name">
              {p.title}
              {p.smart ? " (smart)" : ""}
            </span>
          </button>
        ))
      )}

      <div className="lists-panel-heading">Collections</div>
      {collections.length === 0 ? (
        <div className="lists-panel-empty">
          No collections yet. Add an album to one from its … menu.
        </div>
      ) : (
        collections.map((name) => (
          <button
            key={name}
            className={`lists-panel-row${browseCollectionName === name ? " selected" : ""}`}
            onClick={() => loadAlbumsForCollection(name)}
          >
            <span className="lists-panel-row-name">{name}</span>
          </button>
        ))
      )}

      <div className="lists-panel-heading">Smart Filters</div>
      {bookmarks.length === 0 ? (
        <div className="lists-panel-empty">
          No Smart Filters yet. Set a filter, then tap Save in the filter panel.
        </div>
      ) : (
        bookmarks.map((entry, i) => (
          <button
            key={entry.id}
            className={`lists-panel-row${activeBookmarkName === entry.name ? " selected" : ""}`}
            onClick={() => onLoadBookmark(entry)}
            title={summaries[i]}
          >
            <span className="lists-panel-row-name">{entry.name}</span>
          </button>
        ))
      )}
      {bookmarks.length > 0 && (
        <button className="lists-panel-manage" onClick={onManage}>
          Manage Smart Filters…
        </button>
      )}
    </div>
  );
}

export default function SidebarView({ onOpenSettings }: SidebarProps) {
  const sidebarMode = useLibraryStore((s) => s.sidebarMode);
  const setSidebarMode = useLibraryStore((s) => s.setSidebarMode);
  const artists = useLibraryStore((s) => s.artists);
  const selectedArtistId = useLibraryStore((s) => s.selectedArtistId);
  const selectArtist = useLibraryStore((s) => s.selectArtist);
  const [editorOpen, setEditorOpen] = useState(false);

  useEffect(() => {
    const store = useLibraryStore.getState();
    store.reloadGenreTree();
    store.loadAllAlbums();
    useLibraryStore.setState({ selectedGenreId: "__all__" });
  }, []);

  const loadEntry = useCallback((entry: Bookmark) => {
    useLibraryStore.getState().loadBookmark(filtersFromBookmark(entry), entry.name);
  }, []);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="sidebar-tabs">
        {TABS.map((tab) => (
          <button
            key={tab.mode}
            className={`sidebar-tab${sidebarMode === tab.mode ? " active" : ""}`}
            onClick={() => setSidebarMode(tab.mode)}
          >
            {tab.label}
          </button>
        ))}
      </div>
      <div style={{ flex: 1, overflow: "hidden" }}>
        {sidebarMode === "genres" && <GenreTreeView />}
        {sidebarMode === "artists" && (
          <ArtistList
            artists={artists}
            selectedArtistId={selectedArtistId}
            selectArtist={selectArtist}
          />
        )}
        {sidebarMode === "lists" && (
          <ListsPanel onLoadBookmark={loadEntry} onManage={() => setEditorOpen(true)} />
        )}
      </div>
      <div className="sidebar-bottom-row">
        {onOpenSettings && (
          <button className="sidebar-bottom-btn" onClick={onOpenSettings}>
            Settings
          </button>
        )}
        <button
          className="sidebar-bottom-btn"
          onClick={() => useLibraryStore.getState().loadSuggestion()}
        >
          Suggest
        </button>
      </div>

      {editorOpen && <BookmarkEditor onDismiss={() => setEditorOpen(false)} />}
    </div>
  );
}
