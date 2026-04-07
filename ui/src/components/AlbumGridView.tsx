import { memo, useCallback, useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useLibraryStore, type AlbumSortOrder } from "../stores/libraryStore";
import type { Album } from "../lib/types";
import { getArtUrl } from "../lib/commands";

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

const AlbumCard = memo(function AlbumCard({
  album,
  isSelected,
}: {
  album: Album;
  isSelected: boolean;
}) {
  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artError, setArtError] = useState(false);
  const selectAlbum = useLibraryStore((s) => s.selectAlbum);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const toggleAlbumFav = useLibraryStore((s) => s.toggleAlbumFav);

  useEffect(() => {
    if (!album.thumb) return;
    let cancelled = false;
    getArtUrl(album.thumb, 300)
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
    <div
      className={`album-card${isSelected ? " selected" : ""}`}
      onClick={() => selectAlbum(album)}
      onDoubleClick={() => playAlbum(album)}
    >
      {artSrc && !artError ? (
        <img
          className="album-art"
          src={artSrc}
          alt={album.title}
          loading="lazy"
          onError={() => setArtError(true)}
        />
      ) : (
        <div className="album-art-placeholder">{"\u266B"}</div>
      )}
      <div className="album-title">{album.title}</div>
      <div className="album-artist">{album.artistName}</div>
      <button
        className={`album-fav${album.isFavourite ? " visible" : ""}`}
        onClick={(e) => {
          e.stopPropagation();
          toggleAlbumFav(album);
        }}
      >
        {album.isFavourite ? "\u2605" : "\u2606"}
      </button>
    </div>
  );
});

function useColumnCount(parentRef: React.RefObject<HTMLDivElement | null>): number {
  const [cols, setCols] = useState(4);

  useEffect(() => {
    const el = parentRef.current;
    if (!el) return;
    const compute = () => {
      const contentWidth = el.clientWidth - PAD_H * 2;
      setCols(Math.max(1, Math.floor((contentWidth + GAP) / (MIN_CARD_WIDTH + GAP))));
    };
    compute();
    const obs = new ResizeObserver(compute);
    obs.observe(el);
    return () => obs.disconnect();
  }, [parentRef]);

  return cols;
}

export default function AlbumGridView() {
  const albums = useLibraryStore((s) => s.albums);
  const selectedRatingKey = useLibraryStore(
    (s) => s.selectedAlbum?.ratingKey ?? null
  );
  const albumSortOrder = useLibraryStore((s) => s.albumSortOrder);
  const setAlbumSortOrder = useLibraryStore((s) => s.setAlbumSortOrder);
  const parentRef = useRef<HTMLDivElement>(null);
  const cols = useColumnCount(parentRef);

  const rowCount = Math.ceil(albums.length / cols);

  // Compute exact card width from container
  const [cardWidth, setCardWidth] = useState(MIN_CARD_WIDTH);
  useEffect(() => {
    const el = parentRef.current;
    if (!el) return;
    const compute = () => {
      const contentWidth = el.clientWidth - PAD_H * 2;
      setCardWidth((contentWidth - (cols - 1) * GAP) / cols);
    };
    compute();
    const obs = new ResizeObserver(compute);
    obs.observe(el);
    return () => obs.disconnect();
  }, [cols]);

  const rowHeight = Math.floor(cardWidth + TEXT_HEIGHT + CARD_PAD + GAP);
  const estimateSize = useCallback(() => rowHeight, [rowHeight]);

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
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
    [setAlbumSortOrder]
  );

  if (!albums.length) {
    return <div className="empty-state">Select a genre to browse albums</div>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="album-grid-header">
        <select
          className="sort-select"
          value={albumSortOrder}
          onChange={onSortChange}
        >
          {SORT_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
      </div>
      <div
        ref={parentRef}
        style={{ flex: 1, overflow: "auto" }}
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
                  <div
                    key={album.ratingKey}
                    style={{ width: cardWidth, flexShrink: 0 }}
                  >
                    <AlbumCard
                      album={album}
                      isSelected={album.ratingKey === selectedRatingKey}
                    />
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
