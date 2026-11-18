import { memo, useCallback, useState } from "react";
import { createPortal } from "react-dom";
import type { Album } from "../lib/types";
import { useArtUrl } from "../lib/useArtUrl";
import { useLibraryStore } from "../stores/libraryStore";
import { ART_SIZE } from "../lib/commands";
import { useQueueAlbum } from "../lib/useQueueAlbum";
import { useLongPress } from "../lib/useLongPress";
import { IconMusicNote, IconStarFilled } from "../components/Icons";
import { AlbumDownloadMenuItem } from "../components/DownloadMenuItems";
import CollectionPickerSheet from "./CollectionPickerSheet";
import PlaylistPickerSheet from "./PlaylistPickerSheet";
import { getTracksForAlbum } from "../lib/commands";

interface Props {
  album: Album;
}

export default memo(function MobileAlbumCard({ album }: Props) {
  const openAlbumDetail = useLibraryStore((s) => s.openAlbumDetail);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const loadAlbumsForArtistName = useLibraryStore((s) => s.loadAlbumsForArtistName);
  const { artSrc, artErr, setArtErr } = useArtUrl(album.thumb, ART_SIZE.MEDIUM);

  const [sheetOpen, setSheetOpen] = useState(false);
  const [collectionsOpen, setCollectionsOpen] = useState(false);
  const [playlistOpen, setPlaylistOpen] = useState(false);

  // The shared hold tolerates a few pixels of drift before it reads a touch
  // as a scroll: the hold's own haptic rumble shakes the contact point, and
  // cancelling on any movement at all would let it cancel itself.
  const openSheet = useCallback(() => setSheetOpen(true), []);
  const openDetail = useCallback(() => openAlbumDetail(album), [album, openAlbumDetail]);
  const longPress = useLongPress({ onLongPress: openSheet, onClick: openDetail });

  const queueTracks = useQueueAlbum(album.ratingKey);

  return (
    <>
      <button
        className="mobile-album-card"
        {...longPress}
      >
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
        {/* Always render the year line (its height is reserved in CSS even when
            empty) so every card is a uniform height — the grid virtualizer
            relies on a fixed row size. */}
        <div className="mobile-album-year">{album.year}</div>
      </button>

      {/* Portal to body: the card lives inside a virtualizer row that is
          positioned with a CSS transform, and a transformed ancestor
          becomes the containing block for position:fixed — without the
          portal the "full-screen" backdrop and sheet lay out relative to
          the row (sheet clipped on the top row, darkening cut off at the
          row's bottom edge). */}
      {sheetOpen &&
        createPortal(
          <div
            className="mobile-action-sheet-backdrop"
            onClick={(e) => {
              if (e.target === e.currentTarget) setSheetOpen(false);
            }}
          >
            <div className="mobile-action-sheet">
              <div className="mobile-action-sheet-group">
                <button
                  onClick={() => {
                    setSheetOpen(false);
                    playAlbum(album);
                  }}
                >
                  Play
                </button>
                <button
                  onClick={() => {
                    setSheetOpen(false);
                    queueTracks("next").catch(() => {});
                  }}
                >
                  Play Next
                </button>
                <button
                  onClick={() => {
                    setSheetOpen(false);
                    queueTracks("append").catch(() => {});
                  }}
                >
                  Add to Queue
                </button>
                <button
                  onClick={() => {
                    setSheetOpen(false);
                    void loadAlbumsForArtistName(album.artistName);
                  }}
                >
                  Go to Artist
                </button>
                <button
                  onClick={() => {
                    setSheetOpen(false);
                    setCollectionsOpen(true);
                  }}
                >
                  Add to Collection…
                </button>
                <button
                  onClick={() => {
                    setSheetOpen(false);
                    setPlaylistOpen(true);
                  }}
                >
                  Add to Playlist…
                </button>
                <AlbumDownloadMenuItem
                  albumRatingKey={album.ratingKey}
                  onDone={() => setSheetOpen(false)}
                />
              </div>
              <button className="mobile-action-sheet-cancel" onClick={() => setSheetOpen(false)}>
                Cancel
              </button>
            </div>
          </div>,
          document.body,
        )}
      {collectionsOpen && (
        <CollectionPickerSheet album={album} onDismiss={() => setCollectionsOpen(false)} />
      )}
      {playlistOpen && (
        <PlaylistPickerSheet
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
