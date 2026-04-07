import { useEffect } from "react";
import { useLibraryStore, type SidebarMode } from "../stores/libraryStore";
import GenreTreeView from "./GenreTreeView";
import { useGenreDebugStore } from "./GenreDebugPanel";

const TABS: { mode: SidebarMode; label: string }[] = [
  { mode: "genres", label: "Genres" },
  { mode: "favourites", label: "Favourites" },
  { mode: "artists", label: "Artists" },
];

interface SidebarProps {
  onOpenSettings?: () => void;
}

function ArtistList({ artists, selectedArtistId, selectArtist }: {
  artists: { sourceId: string; name: string }[];
  selectedArtistId: string | null;
  selectArtist: (id: string) => void;
}) {
  const { textSize, padH, rowHeight, chevronWidth } = useGenreDebugStore();

  if (artists.length === 0) {
    return <div className="empty-state">No artists loaded</div>;
  }

  return (
    <div style={{ height: "100%", overflow: "auto", paddingTop: 2 }}>
      {artists.map((artist) => (
        <div
          key={artist.sourceId}
          className={`genre-row${selectedArtistId === artist.sourceId ? " selected" : ""}`}
          style={{
            height: rowHeight,
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
      ))}
    </div>
  );
}

export default function SidebarView({ onOpenSettings }: SidebarProps) {
  const sidebarMode = useLibraryStore((s) => s.sidebarMode);
  const setSidebarMode = useLibraryStore((s) => s.setSidebarMode);
  const artists = useLibraryStore((s) => s.artists);
  const selectedArtistId = useLibraryStore((s) => s.selectedArtistId);
  const selectArtist = useLibraryStore((s) => s.selectArtist);

  // Load genre tree on mount
  useEffect(() => {
    useLibraryStore.getState().loadGenreTree();
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
        {(sidebarMode === "genres" || sidebarMode === "favourites") && (
          <GenreTreeView />
        )}
        {sidebarMode === "artists" && (
          <ArtistList
            artists={artists}
            selectedArtistId={selectedArtistId}
            selectArtist={selectArtist}
          />
        )}
      </div>
      {onOpenSettings && (
        <button className="sidebar-settings-btn" onClick={onOpenSettings}>
          Settings
        </button>
      )}
    </div>
  );
}
