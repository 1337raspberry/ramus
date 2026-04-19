import { memo, useCallback, useEffect, useRef, useState } from "react";
import type { Album } from "../lib/types";
import { useArtUrl } from "../lib/useArtUrl";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { ART_SIZE, appendToQueue, getQueue, getTracksForAlbum, insertNext } from "../lib/commands";
import { IconMusicNote, IconStarFilled } from "../components/Icons";
import { AlbumDownloadMenuItem } from "../components/DownloadMenuItems";

interface Props {
  album: Album;
}

const LONG_PRESS_MS = 500;

export default memo(function MobileAlbumCard({ album }: Props) {
  const openAlbumDetail = useLibraryStore((s) => s.openAlbumDetail);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const { artSrc, artErr, setArtErr } = useArtUrl(album.thumb, ART_SIZE.MEDIUM);

  const [sheetOpen, setSheetOpen] = useState(false);
  const timerRef = useRef<number | null>(null);
  const longPressedRef = useRef(false);

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  useEffect(() => () => clearTimer(), [clearTimer]);

  const handleTouchStart = useCallback(() => {
    longPressedRef.current = false;
    clearTimer();
    timerRef.current = window.setTimeout(() => {
      longPressedRef.current = true;
      setSheetOpen(true);
    }, LONG_PRESS_MS);
  }, [clearTimer]);

  const handleTouchEnd = useCallback(() => {
    clearTimer();
  }, [clearTimer]);

  const handleClick = useCallback(() => {
    if (longPressedRef.current) {
      // Swallow the click that fires after a long-press.
      longPressedRef.current = false;
      return;
    }
    openAlbumDetail(album);
  }, [album, openAlbumDetail]);

  const queueTracks = useCallback(
    async (mode: "next" | "append") => {
      const tracks = await getTracksForAlbum(album.ratingKey);
      const fn = mode === "next" ? insertNext : appendToQueue;
      await fn(tracks);
      const q = await getQueue();
      usePlaybackStore.setState({ queue: q });
    },
    [album.ratingKey],
  );

  return (
    <>
      <button
        className="mobile-album-card"
        onClick={handleClick}
        onTouchStart={handleTouchStart}
        onTouchEnd={handleTouchEnd}
        onTouchMove={handleTouchEnd}
        onTouchCancel={handleTouchEnd}
        onContextMenu={(e) => {
          e.preventDefault();
          setSheetOpen(true);
        }}
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
        {album.year && <div className="mobile-album-year">{album.year}</div>}
      </button>

      {sheetOpen && (
        <div
          className="mobile-action-sheet-backdrop"
          onClick={(e) => {
            if (e.target === e.currentTarget) setSheetOpen(false);
          }}
        >
          <div className="mobile-action-sheet">
            <div className="mobile-action-sheet-header">
              <div className="mobile-action-sheet-title">{album.title}</div>
              <div className="mobile-action-sheet-subtitle">{album.artistName}</div>
            </div>
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
              <AlbumDownloadMenuItem
                albumRatingKey={album.ratingKey}
                onDone={() => setSheetOpen(false)}
              />
            </div>
            <button className="mobile-action-sheet-cancel" onClick={() => setSheetOpen(false)}>
              Cancel
            </button>
          </div>
        </div>
      )}
    </>
  );
});
