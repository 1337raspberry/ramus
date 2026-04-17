import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { Album, GenreNode } from "../lib/types";
import { useLibraryStore } from "../stores/libraryStore";
import { IconChevronLeft, IconChevronDown } from "../components/Icons";
import type { AlbumSortOrder } from "../stores/libraryStore";
import MobileAlbumCard from "./MobileAlbumCard";

function SortIcon({ mode }: { mode: string }) {
  const s = 22;
  switch (mode) {
    case "latestAdded":
      return (
        <svg width={s} height={s} viewBox="0 0 24 24" fill="currentColor">
          <path
            transform="matrix(0.683,0,0,0.683,-0.042,17.084)"
            d="M7.13 2.75L19.36 2.75C18.79 1.86 18.40 0.82 18.29-0.27L7.35-0.27C6.47-0.27 6.01-0.69 6.01-1.62L6.01-12.04C6.01-12.96 6.47-13.38 7.35-13.38L23.23-13.38C24.09-13.38 24.56-12.96 24.56-12.04L24.56-9.16C25.07-9.27 25.57-9.33 26.09-9.33C26.60-9.33 27.11-9.27 27.59-9.16L27.59-15.54C27.59-18.20 26.13-19.65 23.45-19.65L7.13-19.65C4.45-19.65 2.99-18.20 2.99-15.54L2.99-1.36C2.99 1.30 4.45 2.75 7.13 2.75ZM13.08-9.59L13.77-9.59C14.20-9.59 14.36-9.71 14.36-10.15L14.36-10.84C14.36-11.26 14.20-11.40 13.77-11.40L13.08-11.40C12.66-11.40 12.50-11.26 12.50-10.84L12.50-10.15C12.50-9.71 12.66-9.59 13.08-9.59ZM16.80-9.59L17.50-9.59C17.93-9.59 18.07-9.71 18.07-10.15L18.07-10.84C18.07-11.26 17.93-11.40 17.50-11.40L16.80-11.40C16.37-11.40 16.23-11.26 16.23-10.84L16.23-10.15C16.23-9.71 16.37-9.59 16.80-9.59ZM20.53-9.59L21.22-9.59C21.64-9.59 21.80-9.71 21.80-10.15L21.80-10.84C21.80-11.26 21.64-11.40 21.22-11.40L20.53-11.40C20.10-11.40 19.95-11.26 19.95-10.84L19.95-10.15C19.95-9.71 20.10-9.59 20.53-9.59ZM9.35-5.92L10.04-5.92C10.48-5.92 10.63-6.05 10.63-6.48L10.63-7.17C10.63-7.59 10.48-7.73 10.04-7.73L9.35-7.73C8.93-7.73 8.78-7.59 8.78-7.17L8.78-6.48C8.78-6.05 8.93-5.92 9.35-5.92ZM13.08-5.92L13.77-5.92C14.20-5.92 14.36-6.05 14.36-6.48L14.36-7.17C14.36-7.59 14.20-7.73 13.77-7.73L13.08-7.73C12.66-7.73 12.50-7.59 12.50-7.17L12.50-6.48C12.50-6.05 12.66-5.92 13.08-5.92ZM16.80-5.92L17.50-5.92C17.93-5.92 18.07-6.05 18.07-6.48L18.07-7.17C18.07-7.59 17.93-7.73 17.50-7.73L16.80-7.73C16.37-7.73 16.23-7.59 16.23-7.17L16.23-6.48C16.23-6.05 16.37-5.92 16.80-5.92ZM26.10 4.77C29.45 4.77 32.26 1.97 32.26-1.41C32.26-4.78 29.47-7.57 26.10-7.57C22.71-7.57 19.92-4.78 19.92-1.41C19.92 1.98 22.71 4.77 26.10 4.77ZM26.10 2.59C25.52 2.59 25.15 2.21 25.15 1.65L25.15-0.46L23.05-0.46C22.50-0.46 22.11-0.83 22.11-1.39C22.11-1.97 22.48-2.33 23.05-2.33L25.15-2.33L25.15-4.43C25.15-4.98 25.52-5.37 26.10-5.37C26.66-5.37 27.04-5.00 27.04-4.43L27.04-2.33L29.13-2.33C29.70-2.33 30.07-1.97 30.07-1.39C30.07-0.83 29.70-0.46 29.13-0.46L27.04-0.46L27.04 1.65C27.04 2.21 26.66 2.59 26.10 2.59ZM9.35-2.25L10.04-2.25C10.48-2.25 10.63-2.39 10.63-2.81L10.63-3.50C10.63-3.94 10.48-4.07 10.04-4.07L9.35-4.07C8.93-4.07 8.78-3.94 8.78-3.50L8.78-2.81C8.78-2.39 8.93-2.25 9.35-2.25ZM13.08-2.25L13.77-2.25C14.20-2.25 14.36-2.39 14.36-2.81L14.36-3.50C14.36-3.94 14.20-4.07 13.77-4.07L13.08-4.07C12.66-4.07 12.50-3.94 12.50-3.50L12.50-2.81C12.50-2.39 12.66-2.25 13.08-2.25ZM16.80-2.25L17.50-2.25C17.93-2.25 18.07-2.39 18.07-2.81L18.07-3.50C18.07-3.94 17.93-4.07 17.50-4.07L16.80-4.07C16.37-4.07 16.23-3.94 16.23-3.50L16.23-2.81C16.23-2.39 16.37-2.25 16.80-2.25Z"
          />
        </svg>
      );
    case "recentlyPlayed":
      return (
        <svg width={s} height={s} viewBox="0 0 24 24" fill="currentColor">
          <path
            transform="matrix(0.726,0,0,0.726,2.502,18.140)"
            d="M3.68-6.29C4.05-6.29 4.39-6.42 4.72-6.66L8.05-9.07C8.38-9.32 8.58-9.70 8.58-10.11C8.58-10.76 8.07-11.31 7.37-11.31C7.08-11.31 6.80-11.21 6.57-11.00L5.27-9.83C5.94-14.32 9.80-17.75 14.47-17.75C19.61-17.75 23.77-13.59 23.77-8.46C23.77-3.32 19.61 0.84 14.47 0.84C11.19 0.84 8.68-0.86 7.36-2.48C7.00-2.92 6.53-3.12 6.08-3.12C5.32-3.12 4.61-2.52 4.61-1.68C4.61-1.25 4.76-0.79 5.18-0.29C6.87 1.69 10.15 3.93 14.47 3.93C21.30 3.93 26.85-1.63 26.85-8.46C26.85-15.29 21.30-20.84 14.47-20.84C8.37-20.84 3.30-16.42 2.29-10.62L1.56-11.68C1.31-12.04 0.95-12.23 0.54-12.23C-0.13-12.23-0.69-11.78-0.69-11.07C-0.69-10.76-0.60-10.46-0.40-10.22L2.26-7.04C2.70-6.52 3.11-6.29 3.68-6.29ZM11.41-4.61C11.41-4.05 12.05-3.79 12.56-4.09L18.96-7.82C19.43-8.10 19.42-8.79 18.96-9.06L12.56-12.81C12.06-13.09 11.41-12.82 11.41-12.28Z"
          />
        </svg>
      );
    case "random":
      return (
        <svg width={s} height={s} viewBox="0 0 24 24" fill="currentColor">
          <path
            transform="matrix(0.678,0,0,0.678,-0.026,17.648)"
            d="M2.99-1.83C2.99-0.97 3.67-0.33 4.56-0.33L7.45-0.33C9.59-0.33 10.89-0.98 12.21-2.60L15.02-6.02L17.55-2.91C19.21-0.88 20.75-0.27 23.06-0.27L25.20-0.27L25.20 2.38C25.20 3.11 25.64 3.54 26.36 3.54C26.68 3.54 26.96 3.42 27.19 3.23L32.09-0.89C32.63-1.36 32.63-2.10 32.09-2.54L27.19-6.67C26.96-6.86 26.68-6.97 26.36-6.97C25.64-6.97 25.20-6.53 25.20-5.80L25.20-3.29L22.92-3.29C21.43-3.29 20.64-3.73 19.61-4.99L16.89-8.31L19.61-11.63C20.64-12.89 21.43-13.32 22.92-13.32L25.20-13.32L25.20-10.85C25.20-10.13 25.64-9.69 26.36-9.69C26.68-9.69 26.96-9.81 27.19-10.00L32.09-14.12C32.63-14.58 32.63-15.30 32.09-15.77L27.19-19.90C26.96-20.09 26.68-20.20 26.36-20.20C25.64-20.20 25.20-19.76 25.20-19.03L25.20-16.35L23.06-16.35C20.75-16.35 19.21-15.74 17.55-13.71L15.02-10.59L12.21-14.02C10.89-15.63 9.59-16.29 7.45-16.29L4.56-16.29C3.67-16.29 2.99-15.64 2.99-14.79C2.99-13.93 3.67-13.28 4.56-13.28L7.13-13.28C8.60-13.28 9.50-12.79 10.50-11.55L13.15-8.31L10.50-5.06C9.50-3.83 8.60-3.34 7.13-3.34L4.56-3.34C3.67-3.34 2.99-2.68 2.99-1.83Z"
          />
        </svg>
      );
    default:
      return (
        <svg width={s} height={s} viewBox="0 0 24 24" fill="currentColor">
          <path
            transform="matrix(0.676,0,0,0.676,-0.021,17.705)"
            d="M4.84 0.57C5.92 0.57 6.52 0.09 6.82-1.04L7.71-3.81L14.13-3.81L15-1.04C15.30 0.09 15.89 0.57 16.98 0.57C18.15 0.57 18.91-0.14 18.91-1.22C18.91-1.64 18.84-1.99 18.70-2.43L14.05-15.33C13.55-16.79 12.56-17.48 10.93-17.48C9.40-17.48 8.41-16.78 7.91-15.34L3.20-2.29C3.06-1.89 2.99-1.51 2.99-1.17C2.99-0.09 3.69 0.57 4.84 0.57ZM10.90-13.82L10.98-13.82L13.23-6.68L8.63-6.68ZM25.27 0.61C26.99 0.61 28.62-0.28 29.36-1.75L29.37-1.75L29.37-0.84C29.43 0.09 30.04 0.61 30.96 0.61C31.92 0.61 32.57 0.05 32.57-1.02L32.57-8.60C32.57-11.26 30.42-13.02 27.07-13.02C24.39-13.02 22.34-12.02 21.69-10.41C21.59-10.13 21.52-9.86 21.52-9.59C21.52-8.85 22.07-8.33 22.90-8.33C23.47-8.33 23.91-8.54 24.26-8.96C25.01-10.09 25.76-10.56 26.94-10.56C28.39-10.56 29.31-9.76 29.31-8.39L29.31-7.52L25.91-7.34C22.61-7.15 20.80-5.73 20.80-3.36C20.80-1.00 22.68 0.61 25.27 0.61ZM26.24-1.77C24.97-1.77 24.09-2.46 24.09-3.53C24.09-4.55 24.91-5.23 26.36-5.32L29.31-5.52L29.31-4.45C29.31-2.92 27.96-1.77 26.24-1.77Z"
          />
        </svg>
      );
  }
}

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
  const albumSortOrder = useLibraryStore((s) => s.albumSortOrder);
  const setAlbumSortOrder = useLibraryStore((s) => s.setAlbumSortOrder);

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
          <IconChevronLeft size={22} />
        </button>
        <div className="mobile-header-title">
          <span>{title}</span>
          <IconChevronDown size={14} />
        </div>
        <div className="mobile-sort-wrap">
          <SortIcon mode={albumSortOrder} />
          <IconChevronDown size={10} />
          <select
            className="mobile-sort-select"
            value={albumSortOrder}
            onChange={(e) => setAlbumSortOrder(e.target.value as AlbumSortOrder)}
          >
            <option value="alphabetical">A-Z</option>
            <option value="latestAdded">Latest Added</option>
            <option value="recentlyPlayed">Recently Played</option>
            <option value="random">Random</option>
          </select>
        </div>
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
  const [scrolled, setScrolled] = useState(false);
  const rowCount = Math.ceil(albums.length / COLS);
  const estimate = useCallback(() => ROW_HEIGHT, []);
  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: estimate,
    overscan: 2,
  });

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onScroll = () => setScrolled(el.scrollTop > 0);
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, []);

  const topFade = scrolled
    ? ({
        WebkitMaskImage: "linear-gradient(to bottom, transparent, black 36px)",
        maskImage: "linear-gradient(to bottom, transparent, black 36px)",
      } as const)
    : undefined;

  return (
    <div ref={scrollRef} className="mobile-album-grid-scroll" style={topFade}>
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
