import { useCallback, useEffect, useRef, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { pushBackHandler } from "../lib/backHandler";
import { useEdgeSwipeBack } from "./useEdgeSwipeBack";
import { useMediaQuery, SPLIT_QUERY } from "../lib/useMediaQuery";
import MobileToolbar, { type MobileView } from "./MobileToolbar";
import MobileGenreTree from "./MobileGenreTree";
import MobileAlbumGrid from "./MobileAlbumGrid";
import MobileAlbumDetail from "./MobileAlbumDetail";
import MobileArtistList from "./MobileArtistList";
import MobileSuggestion from "./MobileSuggestion";
import MobileSearch from "./MobileSearch";
import MobileNowPlaying from "./MobileNowPlaying";
import MobileListsHub from "./MobileListsHub";
import MobilePlaylistDetail from "./MobilePlaylistDetail";
import GenreInfoSheet from "./GenreInfoSheet";
import type { GenreNode, Playlist } from "../lib/types";

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
  const browseCollectionName = useLibraryStore((s) => s.browseCollectionName);
  const browsePlaylist = useLibraryStore((s) => s.browsePlaylist);
  const hasTrack = usePlaybackStore((s) => !!s.currentTrack);
  const [sheetExpanded, setSheetExpanded] = useState(false);
  // Two-pane browsing on wide touch viewports (an iPad in landscape): the
  // toolbar and its tab surface stay put on the left while albums, album
  // detail and playlists render on the right. Everything below that keys
  // on `split` is about the content pane never being empty and the back
  // ladder bottoming out at the full library instead of the genre tree.
  const split = useMediaQuery(SPLIT_QUERY);
  // Breadcrumb for "Go to Artist" from a playlist: back from that artist
  // grid returns to the playlist (matching the album-detail overlay's back)
  // instead of falling out to the genre tree. Tied to the artist name so a
  // grid that has since navigated elsewhere drops the crumb.
  const [returnPlaylist, setReturnPlaylist] = useState<{
    playlist: Playlist;
    artistName: string;
  } | null>(null);

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

  // Losing the current track (queue cleared, or a stop) unmounts the sheet
  // without collapsing it. Reset here or the next track to play would mount
  // it already expanded, over whatever view the user was on.
  useEffect(() => {
    if (!hasTrack) setSheetExpanded(false);
  }, [hasTrack]);

  // Artist/year navigation (grid long-press, album detail, playlist rows,
  // the now-playing sheet's year link) lands on the album grid — the
  // "lists" view has no surface for these browse contexts, so flip to the
  // grid view when one appears (e.g. Go to Artist from a collection grid
  // or a lists-launched detail). Safe to key on presence: both contexts
  // are transient (cleared by every nav action), so they can't be set
  // while the user is legitimately on the Lists tab.
  // Neither heal applies in two-pane mode: the content pane renders the
  // grid whatever tab is showing, and the "All" promotion below changes
  // the genre selection on every tab — including Lists, which the genre
  // heal would then bounce straight off.
  useEffect(() => {
    if (split) return;
    if ((browseArtistName || browseYear) && view === "lists") setView("genres");
  }, [browseArtistName, browseYear, view, split]);

  // Genre navigation needs the same heal, but keyed on the selection
  // CHANGING, not being set: a genre selection legitimately persists in
  // the store while the user visits the Lists tab, so a presence check
  // (with `view` in the deps) would instantly bounce them back off it.
  // The change-only trigger fires for the now-playing sheet's genre pills
  // and the genre info sheet's drill navigation, which float above every
  // view — without it, a pill tapped over a playlist stranded the user on
  // a toolbar-less Lists hub (selectedGenreId counts toward `inGrid`).
  const viewRef = useRef(view);
  viewRef.current = view;
  useEffect(() => {
    if (split) return;
    if (selectedGenreId && viewRef.current === "lists") setView("genres");
  }, [selectedGenreId, split]);

  // Expanding the player sheet dismisses an active search. The search
  // bar is a NATIVE UISearchBar layered above the webview on iOS, so the
  // sheet cannot cover it — it must be torn down. Clearing searchQuery
  // unmounts MobileSearch, whose cleanup hides the native bar.
  useEffect(() => {
    if (sheetExpanded && useLibraryStore.getState().searchQuery !== null) {
      useLibraryStore.setState({ searchQuery: null });
    }
  }, [sheetExpanded]);

  const miniPlayerVisible = hasTrack;

  // Hide toolbar when drilled into a grid or detail view; keep it on
  // top-level lists (genre tree, artist list, favourite tree). Two-pane
  // keeps it always: the navigation pane is never covered.
  const inGrid =
    !!selectedGenreId ||
    !!browseArtistName ||
    !!browseYear ||
    !!browseCollectionName ||
    !!browsePlaylist ||
    (view === "artists" && !!selectedArtistId);
  const showToolbar =
    split || (!inGrid && !detailAlbum && view !== "search" && view !== "suggestion");

  // Two-pane: the content pane always shows something. With no genre,
  // artist, collection, playlist or album context, the natural resting
  // state is the whole library, so promote an empty selection to "All".
  // Single-pane keeps `null` — there it means "show the tree in place of
  // the grid". Every back step in two-pane mode lands here.
  useEffect(() => {
    if (!split) return;
    const hasContext =
      !!detailAlbum ||
      !!browsePlaylist ||
      !!browseCollectionName ||
      !!browseArtistName ||
      !!browseYear ||
      (view === "artists" && !!selectedArtistId) ||
      !!selectedGenreId;
    if (hasContext) return;
    useLibraryStore.setState({ selectedGenreId: "__all__" });
    useLibraryStore.getState().loadAllAlbums();
  }, [
    split,
    view,
    detailAlbum,
    browsePlaylist,
    browseCollectionName,
    browseArtistName,
    browseYear,
    selectedArtistId,
    selectedGenreId,
  ]);

  // Consume the playlist crumb: leave the artist grid, land back on the
  // playlist. Shared by the unified back handler (hardware/edge back) and
  // the grid header's own chevron (which bypasses handleBack).
  const restorePlaylistFromCrumb = useCallback(() => {
    if (!returnPlaylist) return;
    setReturnPlaylist(null);
    useLibraryStore.setState({
      browseArtistName: null,
      browseYear: null,
      browseCollectionName: null,
      searchQuery: null,
      browsePlaylist: returnPlaylist.playlist,
    });
    setView("lists");
  }, [returnPlaylist]);

  // The crumb only means anything while the grid still shows the artist it
  // was armed from. Retiring it inside the back handler alone is not enough:
  // the grid header's own chevron navigates without going through there, so
  // an abandoned crumb could outlive its context and then re-arm itself on
  // some later, unrelated visit to the same artist — sending that back press
  // to a playlist the user had long since left. `loadAlbumsForArtistName`
  // sets `browseArtistName` synchronously, so this cannot race the arming.
  useEffect(() => {
    if (returnPlaylist && browseArtistName !== returnPlaylist.artistName) {
      setReturnPlaylist(null);
    }
  }, [browseArtistName, returnPlaylist]);

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
      useLibraryStore.setState({ suggestion: null, suggestionMissed: false });
      setView("genres");
      return;
    }

    if (s.browsePlaylist) {
      useLibraryStore.setState({ browsePlaylist: null });
      return;
    }

    if (s.browseCollectionName) {
      // Collection grid (Lists view) → back to the hub. The store list is
      // reloaded so the next grid isn't left showing the collection subset.
      useLibraryStore.setState({ browseCollectionName: null });
      s.loadAllAlbums();
      return;
    }

    if (s.browseArtistName || s.browseYear || s.searchQuery !== null) {
      if (returnPlaylist && s.browseArtistName === returnPlaylist.artistName) {
        restorePlaylistFromCrumb();
        return;
      }
      useLibraryStore.setState({
        browseArtistName: null,
        browseYear: null,
        browseCollectionName: null,
        browsePlaylist: null,
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
      // Two-pane: "All" is the content pane's floor, nothing to pop.
      if (split) return;
      useLibraryStore.setState({ selectedGenreId: null });
      return;
    }

    if (s.selectedGenreId) {
      if (split) {
        // The tree is already on screen — back means "show everything".
        useLibraryStore.setState({ selectedGenreId: "__all__" });
        s.loadAllAlbums();
        return;
      }
      useLibraryStore.setState({ selectedGenreId: null });
      return;
    }
  }, [view, returnPlaylist, restorePlaylistFromCrumb, split]);

  const canGoBack =
    !!detailAlbum ||
    view === "search" ||
    view === "suggestion" ||
    !!browseArtistName ||
    !!browseYear ||
    !!browseCollectionName ||
    !!browsePlaylist ||
    searchQuery !== null ||
    (view === "artists" && !!selectedArtistId) ||
    (split ? !!selectedGenreId && selectedGenreId !== "__all__" : !!selectedGenreId);

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

    if (view === "lists") {
      if (browsePlaylist)
        return (
          <MobilePlaylistDetail
            playlist={browsePlaylist}
            onBack={() => useLibraryStore.setState({ browsePlaylist: null })}
            onGoToArtist={(artistName) => {
              setReturnPlaylist({ playlist: browsePlaylist, artistName });
              void useLibraryStore.getState().loadAlbumsForArtistName(artistName);
              setView("genres");
            }}
          />
        );
      if (browseCollectionName) return <MobileAlbumGrid contextLabel="" />;
      return <MobileListsHub onOpenGrid={() => setView("genres")} />;
    }

    const drillGrid =
      (selectedGenreId && selectedGenreId !== "__all__") || !!browseArtistName || !!browseYear;

    // The grid's header chevron uses its own back ladder, not handleBack —
    // an armed playlist crumb must override it too.
    const crumbActive = returnPlaylist !== null && browseArtistName === returnPlaylist.artistName;

    if (drillGrid)
      return (
        <MobileAlbumGrid
          contextLabel=""
          onBack={crumbActive ? restorePlaylistFromCrumb : undefined}
        />
      );
    if (selectedGenreId === "__all__") return <MobileAlbumGrid contextLabel="All" />;

    return <MobileGenreTree onOpenSettings={onOpenSettings} />;
  };

  // Two-pane: the tab surface for the navigation pane…
  const renderNav = () => {
    if (view === "search" && searchQuery !== null)
      return <MobileSearch onBack={() => setView("genres")} />;
    if (view === "artists") return <MobileArtistList onOpenSettings={onOpenSettings} />;
    if (view === "lists") return <MobileListsHub onOpenGrid={() => setView("genres")} />;
    return <MobileGenreTree onOpenSettings={onOpenSettings} />;
  };

  // …and whatever the current selection resolves to for the content pane.
  // Same precedence as `renderBody`, minus the list surfaces, with the full
  // library as the floor (see the promotion effect above).
  const renderContent = () => {
    if (detailAlbum) return <MobileAlbumDetail />;
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
    if (browsePlaylist)
      return (
        <MobilePlaylistDetail
          playlist={browsePlaylist}
          onBack={() => useLibraryStore.setState({ browsePlaylist: null })}
          onGoToArtist={(artistName) => {
            setReturnPlaylist({ playlist: browsePlaylist, artistName });
            void useLibraryStore.getState().loadAlbumsForArtistName(artistName);
          }}
        />
      );
    if (browseCollectionName) return <MobileAlbumGrid contextLabel="" />;
    if (view === "artists" && selectedArtistId) return <MobileAlbumGrid contextLabel="Artist" />;
    if (browseArtistName || browseYear) {
      const crumbActive = returnPlaylist !== null && browseArtistName === returnPlaylist.artistName;
      return (
        <MobileAlbumGrid
          contextLabel=""
          onBack={crumbActive ? restorePlaylistFromCrumb : undefined}
        />
      );
    }
    if (selectedGenreId && selectedGenreId !== "__all__")
      return <MobileAlbumGrid contextLabel="" />;
    return <MobileAlbumGrid contextLabel="All" hideBack />;
  };

  const rootClass = `mobile-root${miniPlayerVisible ? " with-mini" : ""}${split ? " split" : ""}`;

  return (
    <div ref={containerRef} className={rootClass}>
      {split ? (
        <>
          <aside className="mobile-body mobile-nav-pane">
            <MobileToolbar view={view} onSelect={setView} onOpenSettings={onOpenSettings} />
            {renderNav()}
          </aside>
          <div
            className={`mobile-body mobile-content-pane${view === "suggestion" ? " no-fade" : ""}`}
            style={bodyStyle}
          >
            {renderContent()}
          </div>
        </>
      ) : (
        <>
          {showToolbar && (
            <MobileToolbar view={view} onSelect={setView} onOpenSettings={onOpenSettings} />
          )}
          <div
            className={`mobile-body${view === "suggestion" ? " no-fade" : ""}`}
            style={bodyStyle}
          >
            {renderBody()}
          </div>
        </>
      )}
      {hasTrack && (
        <MobileNowPlaying
          expanded={sheetExpanded}
          onExpand={() => setSheetExpanded(true)}
          onCollapse={() => setSheetExpanded(false)}
          onOpenSettings={() => {
            // The settings panel can't be reached from behind the expanded
            // sheet, so collapse on the way through.
            setSheetExpanded(false);
            onOpenSettings();
          }}
        />
      )}
      <GenreInfoSheet onNavigate={() => setSheetExpanded(false)} />
    </div>
  );
}
