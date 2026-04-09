import { useCallback, useEffect, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useLibraryStore, type SidebarMode } from "../stores/libraryStore";
import GenreTreeView from "./GenreTreeView";
import { useGenreDebugStore } from "./GenreDebugPanel";
import { useSettingsStore } from "../stores/settingsStore";

const TABS: { mode: SidebarMode; label: string }[] = [
  { mode: "genres", label: "Genres" },
  { mode: "favourites", label: "Favourites" },
  { mode: "artists", label: "Artists" },
];

interface SidebarProps {
  onOpenSettings?: () => void;
}

function ArtistList({
  artists,
  selectedArtistId,
  selectArtist,
}: {
  artists: { sourceId: string; name: string }[];
  selectedArtistId: string | null;
  selectArtist: (id: string) => void;
}) {
  const textSize = useGenreDebugStore((s) => s.textSize);
  const padH = useGenreDebugStore((s) => s.padH);
  const rowHeight = useGenreDebugStore((s) => s.rowHeight);
  const chevronWidth = useGenreDebugStore((s) => s.chevronWidth);
  const libraryPadding = useSettingsStore((s) => s.libraryPadding);
  const effectiveRowHeight = Math.max(12, rowHeight + libraryPadding * 2);
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
                paddingLeft: padH,
                paddingRight: padH,
                fontSize: textSize,
                cursor: "pointer",
                whiteSpace: "nowrap",
                overflow: "hidden",
              }}
              onClick={() => selectArtist(artist.sourceId)}
            >
              <span style={{ width: chevronWidth, flexShrink: 0 }} />
              <span className="genre-name">{artist.name}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

export default function SidebarView({ onOpenSettings }: SidebarProps) {
  const sidebarMode = useLibraryStore((s) => s.sidebarMode);
  const setSidebarMode = useLibraryStore((s) => s.setSidebarMode);
  const artists = useLibraryStore((s) => s.artists);
  const selectedArtistId = useLibraryStore((s) => s.selectedArtistId);
  const selectArtist = useLibraryStore((s) => s.selectArtist);

  useEffect(() => {
    const store = useLibraryStore.getState();
    store.loadGenreTree();
    store.loadAllAlbums();
    useLibraryStore.setState({ selectedGenreId: "__all__" });
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
        {(sidebarMode === "genres" || sidebarMode === "favourites") && <GenreTreeView />}
        {sidebarMode === "artists" && (
          <ArtistList
            artists={artists}
            selectedArtistId={selectedArtistId}
            selectArtist={selectArtist}
          />
        )}
      </div>
      <div className="sidebar-bottom-row">
        {onOpenSettings && (
          <button className="sidebar-bottom-btn" onClick={onOpenSettings}>
            Settings
          </button>
        )}
        <button
          className="sidebar-bottom-btn sidebar-lucky-btn"
          onClick={() => useLibraryStore.getState().loadSuggestion()}
        >
          Feelin' Lucky
        </button>
      </div>
    </div>
  );
}
