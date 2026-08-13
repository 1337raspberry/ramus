import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useLibraryStore, hasActiveFilters } from "../stores/libraryStore";
import type { Album } from "../lib/types";
import { ART_SIZE, getTracksForAlbum } from "../lib/commands";
import { useArtUrl } from "../lib/useArtUrl";
import { useQueueAlbum } from "../lib/useQueueAlbum";
import { IconPlay, IconStarFilled, IconStarEmpty, IconMusicNote, IconMoreDots } from "./Icons";
import BreadcrumbBar from "./BreadcrumbBar";
import FilterDropdown from "./FilterDropdown";
import SortDropdown from "./SortDropdown";
import DownloadsHubButton from "./DownloadsHubButton";
import QueueAllButton from "./QueueAllButton";
import { AlbumDownloadMenuItem } from "./DownloadMenuItems";
import CollectionPickerModal from "./CollectionPickerModal";
import PlaylistPickerModal from "./PlaylistPickerModal";

let savedGridScroll = 0;
let savedGridKey = "";

const MIN_CARD_WIDTH = 125;
const GAP = 16;
const PAD_H = 16;
const TEXT_HEIGHT = 40; // title + artist + margins
const CARD_PAD = 8; // 4px top, 4px bottom

const AlbumCard = memo(function AlbumCard({ album }: { album: Album }) {
  const {
    artSrc,
    artErr: artError,
    setArtErr: setArtError,
  } = useArtUrl(album.thumb, ART_SIZE.MEDIUM);
  const [menuOpen, setMenuOpen] = useState(false);
  const [collectionsOpen, setCollectionsOpen] = useState(false);
  const [playlistOpen, setPlaylistOpen] = useState(false);
  const openAlbumDetail = useLibraryStore((s) => s.openAlbumDetail);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const toggleAlbumFav = useLibraryStore((s) => s.toggleAlbumFav);

  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      if (!(e.target as Element).closest(".album-card-menu-wrap")) {
        setMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [menuOpen]);

  const queueTracks = useQueueAlbum(album.ratingKey);

  return (
    <>
      <div className="album-card" onClick={() => openAlbumDetail(album)}>
        <div className="album-art-wrap">
          {artSrc && !artError ? (
            <img
              className="album-art"
              src={artSrc}
              alt={album.title}
              loading="lazy"
              onError={() => setArtError(true)}
            />
          ) : (
            <div className="album-art-placeholder">
              <IconMusicNote />
            </div>
          )}
          <button
            className="album-card-play-btn"
            onClick={(e) => {
              e.stopPropagation();
              playAlbum(album);
            }}
            title="Play"
          >
            <IconPlay />
          </button>
        </div>
        <div className="album-title">{album.title}</div>
        <div className="album-artist">{album.artistName}</div>
        <button
          className={`album-fav${album.isFavourite ? " visible" : ""}`}
          onClick={(e) => {
            e.stopPropagation();
            toggleAlbumFav(album);
          }}
        >
          {album.isFavourite ? <IconStarFilled size="1.5em" /> : <IconStarEmpty size="1.5em" />}
        </button>
        <div className="album-card-menu-wrap">
          <button
            className={`album-card-dots${menuOpen ? " visible" : ""}`}
            onClick={(e) => {
              e.stopPropagation();
              setMenuOpen((v) => !v);
            }}
            title="More actions"
          >
            <IconMoreDots />
          </button>
          {menuOpen && (
            <div className="album-card-dropdown">
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  queueTracks("next").catch(() => {});
                  setMenuOpen(false);
                }}
              >
                Play Next
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  queueTracks("append").catch(() => {});
                  setMenuOpen(false);
                }}
              >
                Add to Queue
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  setMenuOpen(false);
                  setCollectionsOpen(true);
                }}
              >
                Add to Collection…
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  setMenuOpen(false);
                  setPlaylistOpen(true);
                }}
              >
                Add to Playlist…
              </button>
              <AlbumDownloadMenuItem
                albumRatingKey={album.ratingKey}
                onDone={() => setMenuOpen(false)}
              />
            </div>
          )}
        </div>
      </div>
      {/* React siblings of the card, not children: the pickers portal to
        <body>, but synthetic clicks bubble through the REACT tree, so a
        child dialog would fire the card's openAlbumDetail on every inner
        tap (see the MobileFilterPanel save-dialog note). */}
      {collectionsOpen && (
        <CollectionPickerModal album={album} onDismiss={() => setCollectionsOpen(false)} />
      )}
      {playlistOpen && (
        <PlaylistPickerModal
          heading={album.title}
          getTrackIds={() =>
            getTracksForAlbum(album.ratingKey).then((ts) => ts.map((t) => t.ratingKey))
          }
          onDismiss={() => setPlaylistOpen(false)}
        />
      )}
    </>
  );
});

// Uses a callback ref, NOT useRef + useEffect. The empty-state branch
// omits the scroll container, so a useRef-based ResizeObserver would
// attach while `current` is null and — because useRef is stable — the
// effect would never re-run when the container eventually mounts. A
// callback ref fires on every DOM mount/unmount.
function useGridLayout() {
  const [layout, setLayout] = useState({ cols: 4, cardWidth: MIN_CARD_WIDTH });
  const obsRef = useRef<ResizeObserver | null>(null);

  const callbackRef = useCallback((el: HTMLDivElement | null) => {
    obsRef.current?.disconnect();
    if (!el) return;
    const compute = () => {
      const contentWidth = el.clientWidth - PAD_H * 2;
      const cols = Math.max(1, Math.floor((contentWidth + GAP) / (MIN_CARD_WIDTH + GAP)));
      const cardWidth = (contentWidth - (cols - 1) * GAP) / cols;
      setLayout({ cols, cardWidth });
    };
    compute();
    const obs = new ResizeObserver(compute);
    obs.observe(el);
    obsRef.current = obs;
  }, []);

  return { ...layout, callbackRef };
}

export default function AlbumGridView() {
  const albums = useLibraryStore((s) => s.albums);
  const searchQuery = useLibraryStore((s) => s.searchQuery);
  const clearSearchResults = useLibraryStore((s) => s.clearSearchResults);
  const albumFilters = useLibraryStore((s) => s.albumFilters);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [scrolled, setScrolled] = useState(false);
  const { cols, cardWidth, callbackRef } = useGridLayout();

  const gridKey = `${albums.length}:${albums[0]?.ratingKey ?? ""}`;
  const gridKeyRef = useRef(gridKey);
  gridKeyRef.current = gridKey;
  const restoreOffset = gridKey === savedGridKey ? savedGridScroll : 0;

  const setRef = useCallback(
    (el: HTMLDivElement | null) => {
      scrollRef.current = el;
      callbackRef(el);
    },
    [callbackRef],
  );

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && restoreOffset > 0) {
      el.scrollTop = restoreOffset;
    }
    return () => {
      if (el) {
        savedGridScroll = el.scrollTop;
        savedGridKey = gridKeyRef.current;
      }
    };
  }, []);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onScroll = () => setScrolled(el.scrollTop > 0);
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [albums.length]);

  const rowCount = Math.ceil(albums.length / cols);

  const rowHeight = Math.floor(cardWidth + TEXT_HEIGHT + CARD_PAD + GAP);
  const estimateSize = useCallback(() => rowHeight, [rowHeight]);

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize,
    overscan: 3,
    initialOffset: restoreOffset,
  });

  useEffect(() => {
    virtualizer.measure();
  }, [rowHeight, virtualizer]);

  const maskImage = scrolled ? `linear-gradient(to bottom, transparent, black 36px)` : undefined;

  if (!albums.length) {
    return (
      <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
        <div className="album-grid-header">
          <BreadcrumbBar />
          <div className="breadcrumb-right">
            <QueueAllButton />
            <DownloadsHubButton />
            <FilterDropdown />
            <SortDropdown />
          </div>
        </div>
        <div className="empty-state">
          {searchQuery ? (
            <>
              No albums found for &ldquo;{searchQuery}&rdquo;
              <button className="crumb-clear" onClick={clearSearchResults} title="Clear search">
                Clear
              </button>
            </>
          ) : hasActiveFilters(albumFilters) ? (
            "No results found, please reduce your filters"
          ) : (
            "Select a genre to browse albums"
          )}
        </div>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="album-grid-header">
        <BreadcrumbBar />
        <div className="breadcrumb-right">
          <QueueAllButton />
          <DownloadsHubButton />
          <FilterDropdown />
          <SortDropdown />
        </div>
      </div>
      <div
        ref={setRef}
        style={{
          flex: 1,
          overflowY: "auto",
          overflowX: "hidden",
          WebkitMaskImage: maskImage,
          maskImage,
        }}
      >
        <div
          style={{
            height: virtualizer.getTotalSize(),
            width: "100%",
            position: "relative",
          }}
        >
          {virtualizer.getVirtualItems().map((vRow) => {
            const startIdx = vRow.index * cols;
            const rowAlbums = albums.slice(startIdx, startIdx + cols);
            return (
              <div
                key={vRow.index}
                style={{
                  position: "absolute",
                  top: vRow.start,
                  left: PAD_H,
                  right: PAD_H,
                  height: rowHeight - GAP,
                  display: "flex",
                  gap: GAP,
                }}
              >
                {rowAlbums.map((album) => (
                  <div key={album.ratingKey} style={{ width: cardWidth, flexShrink: 0 }}>
                    <AlbumCard album={album} />
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
