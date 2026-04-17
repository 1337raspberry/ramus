import type { Album } from "../lib/types";
import { useArtUrl } from "../lib/useArtUrl";
import { useLibraryStore } from "../stores/libraryStore";
import { ART_SIZE } from "../lib/commands";
import { IconMusicNote, IconStarFilled } from "../components/Icons";

interface Props {
  album: Album;
}

export default function MobileAlbumCard({ album }: Props) {
  const openAlbumDetail = useLibraryStore((s) => s.openAlbumDetail);
  const { artSrc, artErr, setArtErr } = useArtUrl(album.thumb, ART_SIZE.MEDIUM);

  return (
    <button className="mobile-album-card" onClick={() => openAlbumDetail(album)}>
      <div className="mobile-album-art">
        {artSrc && !artErr ? (
          <img src={artSrc} alt={album.title} onError={() => setArtErr(true)} />
        ) : (
          <div className="mobile-album-art-ph">
            <IconMusicNote size={32} />
          </div>
        )}
        {album.isFavourite && (
          <span className="mobile-album-fav">
            <IconStarFilled size={16} />
          </span>
        )}
      </div>
      <div className="mobile-album-title" title={album.title}>
        {album.title}
      </div>
      <div className="mobile-album-artist" title={album.artistName}>
        {album.artistName}
      </div>
      {album.year && <div className="mobile-album-year">{album.year}</div>}
    </button>
  );
}
