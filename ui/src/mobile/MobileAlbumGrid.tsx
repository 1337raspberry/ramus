import { useCallback, useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Album, GenreNode } from "../lib/types";
import { useLibraryStore } from "../stores/libraryStore";
import { IconChevronLeft, IconShuffle, IconChevronDown } from "../components/Icons";
import MobileAlbumCard from "./MobileAlbumCard";

const COLS = 3;
/** Approximate row height; the virtualizer re-measures once rendered. */
const ROW_HEIGHT = 210;

function findNode(nodes: GenreNode[], id: string): GenreNode | null {
  for (const n of nodes) {
    if (n.id === id) return n;
    const hit = n.children ? findNode(n.children, id) : null;
    if (hit) return hit;
  }
  return null;
}

interface Props {
  /** Fallback title when no genre/artist/year context is active. */
  contextLabel: string;
}

/**
 * 3-column album grid with back + title + shuffle header. Title is the
 * current genre name (or artist, year, search query, "Favourites", "All").
 */
export default function MobileAlbumGrid({ contextLabel }: Props) {
  const albums = useLibraryStore((s) => s.albums);
  const sidebarMode = useLibraryStore((s) => s.sidebarMode);
  const selectedGenreId = useLibraryStore((s) => s.selectedGenreId);
  const genreTree = useLibraryStore((s) => s.genreTree);
  const browseArtistName = useLibraryStore((s) => s.browseArtistName);
  const browseYear = useLibraryStore((s) => s.browseYear);
  const searchQuery = useLibraryStore((s) => s.searchQuery);
  const artists = useLibraryStore((s) => s.artists);
  const selectedArtistId = useLibraryStore((s) => s.selectedArtistId);
  const shuffleAlbums = useLibraryStore((s) => s.shuffleAlbums);

  const title = useMemo(() => {
    if (searchQuery) return `"${searchQuery}"`;
    if (browseArtistName) return browseArtistName;
    if (browseYear) return String(browseYear);
    if (sidebarMode === "artists" && selectedArtistId) {
      return artists.find((a) => a.sourceId === selectedArtistId)?.name ?? "Artist";
    }
    if (selectedGenreId && selectedGenreId !== "__all__") {
      const node = findNode(genreTree, selectedGenreId);
      if (node) return node.name;
      return selectedGenreId;
    }
    return contextLabel || "All";
  }, [
    searchQuery,
    browseArtistName,
    browseYear,
    sidebarMode,
    selectedArtistId,
    artists,
    selectedGenreId,
    genreTree,
    contextLabel,
  ]);

  const handleBack = () => {
    // Pop one level: drop browse context first, then genre selection, then
    // fall back to the top-level tree view by clearing the selected genre.
    const store = useLibraryStore.getState();
    if (browseArtistName || browseYear || searchQuery !== null) {
      useLibraryStore.setState({
        browseArtistName: null,
        browseYear: null,
        searchQuery: null,
      });
      // Reload the underlying view.
      if (selectedGenreId === "__all__" || !selectedGenreId) {
        if (sidebarMode === "favourites") store.loadFavouriteAlbums();
        else store.loadAllAlbums();
      } else {
        const node = findNode(genreTree, selectedGenreId);
        if (node) store.selectGenre(node);
      }
      return;
    }
    if (sidebarMode === "artists" && selectedArtistId) {
      useLibraryStore.setState({ selectedArtistId: null, albums: [] });
      return;
    }
    if (selectedGenreId === "__all__") {
      useLibraryStore.setState({ selectedGenreId: null });
      return;
    }
    if (selectedGenreId) {
      useLibraryStore.setState({ selectedGenreId: null });
    }
  };

  return (
    <div className="mobile-screen">
      <header className="mobile-header">
        <button className="mobile-header-circle" onClick={handleBack} aria-label="Back">
          <IconChevronLeft />
        </button>
        <div className="mobile-header-title">
          <span>{title}</span>
          <IconChevronDown size={14} />
        </div>
        <button
          className="mobile-header-circle accent"
          onClick={shuffleAlbums}
          aria-label="Shuffle order"
        >
          <IconShuffle />
        </button>
      </header>

      {albums.length === 0 ? (
        <div className="mobile-empty">No albums</div>
      ) : (
        <VirtualizedAlbumGrid albums={albums} />
      )}
    </div>
  );
}

/**
 * Row-virtualized 3-column grid. Only visible rows mount their
 * MobileAlbumCard children, which means only visible cards fire
 * `getArtUrl` IPC calls. Without this, a 2,255-album library fires
 * 2,255 parallel IPC + Plex fetches at mount, swamps the bridge,
 * and leaks memory in the WKWebView image cache.
 */
function VirtualizedAlbumGrid({ albums }: { albums: Album[] }) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const rowCount = Math.ceil(albums.length / COLS);
  const estimate = useCallback(() => ROW_HEIGHT, []);
  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: estimate,
    overscan: 2,
  });

  return (
    <div ref={scrollRef} className="mobile-album-grid-scroll">
      <div
        style={{
          height: virtualizer.getTotalSize(),
          position: "relative",
          width: "100%",
        }}
      >
        {virtualizer.getVirtualItems().map((row) => {
          const start = row.index * COLS;
          const rowAlbums = albums.slice(start, start + COLS);
          return (
            <div
              key={row.key}
              className="mobile-album-grid-row"
              style={{ transform: `translateY(${row.start}px)` }}
              ref={virtualizer.measureElement}
              data-index={row.index}
            >
              {rowAlbums.map((album) => (
                <MobileAlbumCard key={album.ratingKey} album={album} />
              ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}
