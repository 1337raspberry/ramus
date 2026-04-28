import { useCallback, useEffect, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { pushBackHandler } from "../lib/backHandler";
import { useEdgeSwipeBack } from "./useEdgeSwipeBack";
import MobileToolbar, { type MobileView } from "./MobileToolbar";
import MobileGenreTree from "./MobileGenreTree";
import MobileAlbumGrid from "./MobileAlbumGrid";
import MobileAlbumDetail from "./MobileAlbumDetail";
import MobileArtistList from "./MobileArtistList";
import MobileSuggestion from "./MobileSuggestion";
import MobileSearch from "./MobileSearch";
import MobileNowPlaying from "./MobileNowPlaying";
import type { GenreNode } from "../lib/types";

function findNode(nodes: GenreNode[], id: string): GenreNode | null {
  for (const n of nodes) {
    if (n.id === id) return n;
    const hit = n.children ? findNode(n.children, id) : null;
    if (hit) return hit;
  }
  return null;
}

interface Props {
  onOpenSettings: () => void;
}

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
  const hasTrack = usePlaybackStore((s) => !!s.currentTrack);
  const [sheetExpanded, setSheetExpanded] = useState(false);

  useEffect(() => {
    const store = useLibraryStore.getState();
    store.reloadGenreTree();
    store.loadAllAlbums();
  }, []);

  useEffect(() => {
    if (sidebarMode === "artists") setView("artists");
    else setView("genres");
  }, [sidebarMode]);

  useEffect(() => {
    if (suggestion) setView("suggestion");
  }, [suggestion]);

  useEffect(() => {
    if (searchQuery !== null) setView("search");
    else setView((v) => (v === "search" ? "genres" : v));
  }, [searchQuery]);

  useEffect(() => {
    if (!sheetExpanded) {
      usePlaybackStore.setState({ isFocusMode: false });
    }
  }, [sheetExpanded]);

  const miniPlayerVisible = hasTrack;

  // Hide toolbar when drilled into a grid or detail view; keep it on
  // top-level lists (genre tree, artist list, favourite tree).
  const inGrid =
    !!selectedGenreId ||
    !!browseArtistName ||
    !!browseYear ||
    (view === "artists" && !!selectedArtistId);
  const showToolbar = !inGrid && !detailAlbum && view !== "search" && view !== "suggestion";

  // Unified back navigation — pops one level of the view hierarchy
  const handleBack = useCallback(() => {
    const s = useLibraryStore.getState();

    if (s.detailAlbum) {
      s.closeAlbumDetail();
      return;
    }

    if (view === "search") {
      useLibraryStore.setState({ searchQuery: null });
      setView("genres");
      return;
    }

    if (view === "suggestion") {
      useLibraryStore.setState({ suggestion: null });
      setView("genres");
      return;
    }

    if (s.browseArtistName || s.browseYear || s.searchQuery !== null) {
      useLibraryStore.setState({
        browseArtistName: null,
        browseYear: null,
        searchQuery: null,
      });
      const gid = s.selectedGenreId;
      if (!gid || gid === "__all__") {
        s.loadAllAlbums();
      } else {
        const node = findNode(s.genreTree, gid);
        if (node) s.selectGenre(node);
      }
      return;
    }

    if (view === "artists" && s.selectedArtistId) {
      useLibraryStore.setState({ selectedArtistId: null, albums: [] });
      return;
    }

    if (s.selectedGenreId === "__all__") {
      useLibraryStore.setState({ selectedGenreId: null });
      return;
    }

    if (s.selectedGenreId) {
      useLibraryStore.setState({ selectedGenreId: null });
      return;
    }
  }, [view]);

  const canGoBack =
    !!detailAlbum ||
    view === "search" ||
    view === "suggestion" ||
    !!browseArtistName ||
    !!browseYear ||
    searchQuery !== null ||
    (view === "artists" && !!selectedArtistId) ||
    !!selectedGenreId;

  useEffect(() => {
    return pushBackHandler(() => {
      if (sheetExpanded) {
        setSheetExpanded(false);
        return true;
      }
      if (canGoBack) {
        handleBack();
        return true;
      }
      return false;
    });
  }, [sheetExpanded, canGoBack, handleBack]);

  const { containerRef, swipeX } = useEdgeSwipeBack(handleBack, canGoBack && !sheetExpanded);

  const bodyStyle =
    swipeX > 0
      ? { transform: `translateX(${swipeX}px)`, opacity: Math.max(0.6, 1 - swipeX / 300) }
      : undefined;

  const renderBody = () => {
    if (detailAlbum) return <MobileAlbumDetail />;

    if (view === "search" && searchQuery !== null)
      return <MobileSearch onBack={() => setView("genres")} />;
    if (view === "suggestion")
      return (
        <MobileSuggestion
          onClose={() => setView("genres")}
          onPlay={() => {
            setView("genres");
            setSheetExpanded(true);
          }}
        />
      );

    if (view === "artists") {
      if (selectedArtistId) return <MobileAlbumGrid contextLabel="Artist" />;
      if (browseArtistName) return <MobileAlbumGrid contextLabel="" />;
      return <MobileArtistList onOpenSettings={onOpenSettings} />;
    }

    const drillGrid =
      (selectedGenreId && selectedGenreId !== "__all__") || !!browseArtistName || !!browseYear;

    if (drillGrid) return <MobileAlbumGrid contextLabel="" />;
    if (selectedGenreId === "__all__") return <MobileAlbumGrid contextLabel="All" />;

    return <MobileGenreTree onOpenSettings={onOpenSettings} />;
  };

  return (
    <div ref={containerRef} className={`mobile-root${miniPlayerVisible ? " with-mini" : ""}`}>
      {showToolbar && (
        <MobileToolbar view={view} onSelect={setView} onOpenSettings={onOpenSettings} />
      )}
      <div className={`mobile-body${view === "suggestion" ? " no-fade" : ""}`} style={bodyStyle}>
        {renderBody()}
      </div>
      {hasTrack && (
        <MobileNowPlaying
          expanded={sheetExpanded}
          onExpand={() => setSheetExpanded(true)}
          onCollapse={() => setSheetExpanded(false)}
        />
      )}
    </div>
  );
}
