import { useEffect, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import MobileToolbar, { type MobileView } from "./MobileToolbar";
import MobileGenreTree from "./MobileGenreTree";
import MobileAlbumGrid from "./MobileAlbumGrid";
import MobileAlbumDetail from "./MobileAlbumDetail";
import MobileArtistList from "./MobileArtistList";
import MobileSuggestion from "./MobileSuggestion";
import MobileSearch from "./MobileSearch";
import MobileNowPlaying from "./MobileNowPlaying";

interface Props {
  onOpenSettings: () => void;
}

/**
 * Root of the stacked mobile layout. Picks one of:
 *   - genre tree / favourite genre tree (toolbar tabs 1/2)
 *   - artist list (toolbar tab 3)
 *   - suggestion card (toolbar button 4)
 *   - search page (toolbar button 7)
 * and overlays the now-playing sheet / mini-player when a track is playing.
 *
 * Navigation is driven by libraryStore state (selectedGenreId, detailAlbum,
 * suggestion, searchQuery) plus a local `view` selector for the toolbar
 * mode. Back buttons set the appropriate store state to `null`.
 */
export default function MobileApp({ onOpenSettings }: Props) {
  const [view, setView] = useState<MobileView>("genres");
  const sidebarMode = useLibraryStore((s) => s.sidebarMode);
  const detailAlbum = useLibraryStore((s) => s.detailAlbum);
  const suggestion = useLibraryStore((s) => s.suggestion);
  const searchQuery = useLibraryStore((s) => s.searchQuery);
  const selectedGenreId = useLibraryStore((s) => s.selectedGenreId);
  const selectedArtistId = useLibraryStore((s) => s.selectedArtistId);
  const browseArtistName = useLibraryStore((s) => s.browseArtistName);
  const browseYear = useLibraryStore((s) => s.browseYear);
  const currentTrack = usePlaybackStore((s) => s.currentTrack);
  const [sheetExpanded, setSheetExpanded] = useState(false);

  useEffect(() => {
    const store = useLibraryStore.getState();
    store.loadGenreTree();
    store.loadAllAlbums();
  }, []);

  // Keep the toolbar view in sync with navigation that happens from
  // elsewhere (e.g. tapping a genre chip in now-playing).
  useEffect(() => {
    if (sidebarMode === "favourites") setView("favourites");
    else if (sidebarMode === "artists") setView("artists");
    else setView("genres");
  }, [sidebarMode]);

  useEffect(() => {
    if (suggestion) setView("suggestion");
  }, [suggestion]);

  useEffect(() => {
    if (searchQuery !== null) setView("search");
  }, [searchQuery]);

  // When the user collapses the sheet (e.g. by pulling down to the mini
  // player), drop out of focus mode so desktop visualizer logic doesn't
  // run in the background.
  useEffect(() => {
    if (!sheetExpanded) {
      usePlaybackStore.setState({ isFocusMode: false });
    }
  }, [sheetExpanded]);

  const miniPlayerVisible = !!currentTrack && !sheetExpanded;

  const renderBody = () => {
    // Album detail takes priority when set, regardless of the selected tab.
    if (detailAlbum) return <MobileAlbumDetail />;

    if (view === "search") return <MobileSearch onBack={() => setView("genres")} />;
    if (view === "suggestion") return <MobileSuggestion onClose={() => setView("genres")} />;

    if (view === "artists") {
      if (selectedArtistId) return <MobileAlbumGrid contextLabel="Artist" />;
      return <MobileArtistList onOpenSettings={onOpenSettings} />;
    }

    // Genres or favourites: drill from tree into a grid when a specific
    // genre is chosen, or when the user drilled into an artist/year from
    // a now-playing chip.
    const inGrid =
      (selectedGenreId && selectedGenreId !== "__all__") || !!browseArtistName || !!browseYear;

    if (inGrid) return <MobileAlbumGrid contextLabel="" />;
    if (selectedGenreId === "__all__") return <MobileAlbumGrid contextLabel="All" />;

    return <MobileGenreTree onOpenSettings={onOpenSettings} />;
  };

  return (
    <div className={`mobile-root${miniPlayerVisible ? " with-mini" : ""}`}>
      <MobileToolbar view={view} onSelect={setView} onOpenSettings={onOpenSettings} />
      <div className="mobile-body">{renderBody()}</div>

      {currentTrack && (
        <MobileNowPlaying
          expanded={sheetExpanded}
          onExpand={() => setSheetExpanded(true)}
          onCollapse={() => setSheetExpanded(false)}
        />
      )}
    </div>
  );
}
