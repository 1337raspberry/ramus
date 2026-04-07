import { useCallback, useState } from "react";
import { useLibraryStore, type AlbumSortOrder } from "../stores/libraryStore";
import type { Album } from "../lib/types";
import { getArtUrl } from "../lib/commands";

const SORT_OPTIONS: { value: AlbumSortOrder; label: string }[] = [
  { value: "alphabetical", label: "A-Z" },
  { value: "latestAdded", label: "Latest Added" },
  { value: "recentlyPlayed", label: "Recently Played" },
  { value: "random", label: "Random" },
];

function AlbumCard({ album }: { album: Album }) {
  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artError, setArtError] = useState(false);
  const selectedAlbum = useLibraryStore((s) => s.selectedAlbum);
  const selectAlbum = useLibraryStore((s) => s.selectAlbum);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const toggleAlbumFav = useLibraryStore((s) => s.toggleAlbumFav);
  const isSelected = selectedAlbum?.ratingKey === album.ratingKey;

  // Load art URL lazily
  if (album.thumb && !artSrc && !artError) {
    getArtUrl(album.thumb, 300)
      .then(setArtSrc)
      .catch(() => setArtError(true));
  }

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
        <div className="album-art-placeholder">♫</div>
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
        {album.isFavourite ? "★" : "☆"}
      </button>
    </div>
  );
}

export default function AlbumGridView() {
  const albums = useLibraryStore((s) => s.albums);
  const albumSortOrder = useLibraryStore((s) => s.albumSortOrder);
  const setAlbumSortOrder = useLibraryStore((s) => s.setAlbumSortOrder);

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
    <div>
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
      <div className="album-grid">
        {albums.map((album) => (
          <AlbumCard key={album.ratingKey} album={album} />
        ))}
      </div>
    </div>
  );
}
