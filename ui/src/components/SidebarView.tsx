import { useEffect } from "react";
import { useLibraryStore, type SidebarMode } from "../stores/libraryStore";
import GenreTreeView from "./GenreTreeView";

const TABS: { mode: SidebarMode; label: string }[] = [
  { mode: "genres", label: "Genres" },
  { mode: "favourites", label: "Favourites" },
  { mode: "artists", label: "Artists" },
];

export default function SidebarView() {
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
          <div style={{ height: "100%", overflow: "auto" }}>
            {artists.length === 0 ? (
              <div className="empty-state">No artists loaded</div>
            ) : (
              artists.map((artist) => (
                <div
                  key={artist.sourceId}
                  className={`genre-row${selectedArtistId === artist.sourceId ? " selected" : ""}`}
                  onClick={() => selectArtist(artist.sourceId)}
                >
                  <span className="genre-name">{artist.name}</span>
                </div>
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
}
