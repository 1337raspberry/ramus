import { memo, useCallback, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useLibraryStore, type AlbumSortOrder } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import type { Album } from "../lib/types";
import { ART_SIZE, getArtUrl, getFavouriteTracks, playTracks, getQueue } from "../lib/commands";
import { IconPlay, IconStarFilled, IconStarEmpty, IconMusicNote, IconShuffle } from "./Icons";

const SORT_OPTIONS: { value: AlbumSortOrder; label: string }[] = [
  { value: "alphabetical", label: "A-Z" },
  { value: "latestAdded", label: "Latest Added" },
  { value: "recentlyPlayed", label: "Recently Played" },
  { value: "random", label: "Random" },
];

const MIN_CARD_WIDTH = 125;
const GAP = 16;
const PAD_H = 16;
const TEXT_HEIGHT = 40; // title + artist + margins
const CARD_PAD = 8; // 4px top + 4px bottom

const AlbumCard = memo(function AlbumCard({ album }: { album: Album }) {
  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artError, setArtError] = useState(false);
  const openAlbumDetail = useLibraryStore((s) => s.openAlbumDetail);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const toggleAlbumFav = useLibraryStore((s) => s.toggleAlbumFav);

  useEffect(() => {
    if (!album.thumb) return;
    let cancelled = false;
    getArtUrl(album.thumb, ART_SIZE.MEDIUM)
      .then((url) => {
        if (!cancelled) setArtSrc(url);
      })
      .catch(() => {
        if (!cancelled) setArtError(true);
      });
    return () => {
      cancelled = true;
    };
  }, [album.thumb]);

  return (
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
    </div>
  );
});

// IMPORTANT: This uses a callback ref, NOT useRef + useEffect.
// When albums are empty the grid renders an empty-state div and the scroll
// container is not in the DOM at all. A useRef-based ResizeObserver would
// attach during the empty state (parentRef.current === null), and because
// useRef is a stable reference the useEffect dependency never changes, so
// the observer is never re-attached when the scroll container finally mounts.
// A callback ref fires every time the DOM element mounts/unmounts, ensuring
// the ResizeObserver is always set up correctly regardless of render timing.
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
  const albumSortOrder = useLibraryStore((s) => s.albumSortOrder);
  const setAlbumSortOrder = useLibraryStore((s) => s.setAlbumSortOrder);
  const sidebarMode = useLibraryStore((s) => s.sidebarMode);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const { cols, cardWidth, callbackRef } = useGridLayout();

  const setRef = useCallback(
    (el: HTMLDivElement | null) => {
      scrollRef.current = el;
      callbackRef(el);
    },
    [callbackRef],
  );

  const rowCount = Math.ceil(albums.length / cols);

  const rowHeight = Math.floor(cardWidth + TEXT_HEIGHT + CARD_PAD + GAP);
  const estimateSize = useCallback(() => rowHeight, [rowHeight]);

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize,
    overscan: 3,
  });

  useEffect(() => {
    virtualizer.measure();
  }, [rowHeight, virtualizer]);

  const onSortChange = useCallback(
    (e: React.ChangeEvent<HTMLSelectElement>) => {
      setAlbumSortOrder(e.target.value as AlbumSortOrder);
    },
    [setAlbumSortOrder],
  );

  const handleShuffleFavs = useCallback(() => {
    getFavouriteTracks()
      .then((tracks) => {
        if (!tracks.length) return;
        // Fisher-Yates shuffle
        const shuffled = [...tracks];
        for (let i = shuffled.length - 1; i > 0; i--) {
          const j = Math.floor(Math.random() * (i + 1));
          [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]];
        }
        return playTracks(shuffled, 0).then(() => getQueue());
      })
      .then((q) => {
        if (q) usePlaybackStore.setState({ queue: q });
      })
      .catch(() => {});
  }, []);

  if (!albums.length) {
    return <div className="empty-state">Select a genre to browse albums</div>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="album-grid-header">
        {sidebarMode === "favourites" && (
          <button
            className="shuffle-favs-btn"
            onClick={handleShuffleFavs}
            title="Shuffle all favourite tracks"
          >
            <IconShuffle size={14} />
            <span>Shuffle</span>
          </button>
        )}
        <select className="sort-select" value={albumSortOrder} onChange={onSortChange}>
          {SORT_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
      </div>
      <div ref={setRef} style={{ flex: 1, overflowY: "auto", overflowX: "hidden" }}>
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
