import { useCallback, useEffect, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { usePlaybackStore } from "../stores/playbackStore";
import { useArtUrl } from "../lib/useArtUrl";
import { ART_SIZE, getAlbumGenres, insertNext, appendToQueue, getQueue } from "../lib/commands";
import { formatDuration, formatCodec } from "../lib/format";
import {
  IconChevronLeft,
  IconStarFilled,
  IconStarEmpty,
  IconMusicNote,
  IconPlay,
  IconMoreDots,
} from "../components/Icons";
import FlowLayout from "../components/FlowLayout";
import MarqueeText from "../components/MarqueeText";
import { AlbumDownloadMenuItem, TrackDownloadMenuItem } from "../components/DownloadMenuItems";

/**
 * Album detail: hero art + artist/year/genres, then track list. Reuses the
 * desktop action verbs (toggleAlbumFav, playAlbum, ...) so search/queue
 * semantics stay identical between mobile and desktop.
 */
export default function MobileAlbumDetail() {
  const album = useLibraryStore((s) => s.detailAlbum);
  const tracks = useLibraryStore((s) => s.detailTracks);
  const closeAlbumDetail = useLibraryStore((s) => s.closeAlbumDetail);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const toggleAlbumFav = useLibraryStore((s) => s.toggleAlbumFav);
  const toggleTrackFav = useLibraryStore((s) => s.toggleTrackFav);
  const loadAlbumsForArtistName = useLibraryStore((s) => s.loadAlbumsForArtistName);
  const loadAlbumsForYear = useLibraryStore((s) => s.loadAlbumsForYear);
  const selectGenreByName = useLibraryStore((s) => s.selectGenreByName);

  const { artSrc, artErr, setArtErr } = useArtUrl(album?.thumb, ART_SIZE.MEDIUM);
  const [genres, setGenres] = useState<string[]>([]);
  const [openMenuKey, setOpenMenuKey] = useState<string | null>(null);

  useEffect(() => {
    if (!openMenuKey) return;
    const handleTap = (e: MouseEvent | TouchEvent) => {
      const target = e.target as HTMLElement;
      if (!target.closest("[data-menu-wrap]")) {
        setOpenMenuKey(null);
      }
    };
    document.addEventListener("touchstart", handleTap);
    document.addEventListener("mousedown", handleTap);
    return () => {
      document.removeEventListener("touchstart", handleTap);
      document.removeEventListener("mousedown", handleTap);
    };
  }, [openMenuKey]);

  const albumKey = album?.ratingKey;
  const inlineGenres = album?.genres;
  useEffect(() => {
    if (!albumKey || !inlineGenres) {
      setGenres([]);
      return;
    }
    if (inlineGenres.length) {
      setGenres(inlineGenres);
      return;
    }
    let cancelled = false;
    getAlbumGenres(albumKey)
      .then((g) => {
        if (!cancelled) setGenres(g);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [albumKey, inlineGenres]);

  const handleGenreClick = useCallback(
    (g: string) => {
      closeAlbumDetail();
      selectGenreByName(g);
    },
    [closeAlbumDetail, selectGenreByName],
  );

  const queueAction = useCallback((fn: typeof insertNext, items: typeof tracks) => {
    fn(items)
      .then(() => getQueue())
      .then((q) => usePlaybackStore.setState({ queue: q }))
      .catch(() => {});
    setOpenMenuKey(null);
  }, []);

  if (!album) return null;

  const codec = tracks.length ? formatCodec(tracks[0].codec, tracks[0].bitrate) : null;
  const totalMinutes = Math.round(tracks.reduce((s, t) => s + t.duration, 0) / 60);
  const hasMultipleDiscs = tracks.some((t) => (t.discNumber ?? 1) > 1);

  return (
    <div className="mobile-screen">
      <header className="mobile-header">
        <button className="mobile-header-circle" onClick={closeAlbumDetail} aria-label="Back">
          <IconChevronLeft size={22} />
        </button>
        <MarqueeText className="mobile-header-title mobile-header-title-short">
          {album.title}
        </MarqueeText>
        <button
          className={`mobile-header-circle${album.isFavourite ? " accent" : ""}`}
          onClick={() => toggleAlbumFav(album)}
          aria-label={album.isFavourite ? "Remove favourite" : "Add favourite"}
        >
          {album.isFavourite ? <IconStarFilled size={22} /> : <IconStarEmpty size={22} />}
        </button>
      </header>

      <div className="mobile-detail-body">
        <div className="mobile-detail-hero">
          <div className="mobile-detail-art">
            {artSrc && !artErr ? (
              <img src={artSrc} alt={album.title} onError={() => setArtErr(true)} />
            ) : (
              <div className="mobile-detail-art-ph">
                <IconMusicNote size={32} />
              </div>
            )}
          </div>
          <div className="mobile-detail-meta">
            <div
              className="mobile-detail-artist"
              onClick={() => loadAlbumsForArtistName(album.artistName)}
            >
              {album.artistName}
            </div>
            {album.year && (
              <div className="mobile-detail-year" onClick={() => loadAlbumsForYear(album.year!)}>
                {album.year}
              </div>
            )}
            <div className="mobile-detail-summary">
              {tracks.length} {tracks.length === 1 ? "track" : "tracks"} &middot; {totalMinutes} min
            </div>
            <div className="mobile-detail-actions">
              <button
                className="mobile-detail-play"
                aria-label="Play album"
                onClick={() => playAlbum(album)}
              >
                <IconPlay size={22} />
              </button>
              <div className="mobile-menu-wrap" data-menu-wrap>
                <button
                  className="mobile-dots"
                  onClick={() =>
                    setOpenMenuKey((prev) => (prev === "__album__" ? null : "__album__"))
                  }
                  aria-label="More actions"
                >
                  <IconMoreDots size={20} />
                </button>
                {openMenuKey === "__album__" && (
                  <div className="mobile-dropdown">
                    <button onClick={() => queueAction(insertNext, tracks)}>Play Next</button>
                    <button onClick={() => queueAction(appendToQueue, tracks)}>Add to Queue</button>
                    <AlbumDownloadMenuItem
                      albumRatingKey={album.ratingKey}
                      onDone={() => setOpenMenuKey(null)}
                    />
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
        {genres.length > 0 && (
          <div className="mobile-detail-genres">
            <FlowLayout genres={genres} onGenreClick={handleGenreClick} />
          </div>
        )}

        <ul className="mobile-track-list">
          {tracks.map((t, i) => {
            const showDiscHeader =
              hasMultipleDiscs && (i === 0 || t.discNumber !== tracks[i - 1].discNumber);
            const hasTrackArtist =
              t.trackArtist && t.trackArtist.toLowerCase() !== album.artistName.toLowerCase();
            return (
              <li key={t.ratingKey} className="mobile-track-li">
                {showDiscHeader && (
                  <div className="mobile-disc-header">Disc {t.discNumber ?? 1}</div>
                )}
                <div
                  role="button"
                  tabIndex={0}
                  className="mobile-track-row"
                  onClick={() => playAlbum(album, i)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      playAlbum(album, i);
                    }
                  }}
                >
                  <span className="mobile-track-num">{t.index ?? i + 1}</span>
                  <div className="mobile-track-info">
                    <div className="mobile-track-title">{t.title}</div>
                    {hasTrackArtist && <div className="mobile-track-artist">{t.trackArtist}</div>}
                  </div>
                  <span className="mobile-track-duration">{formatDuration(t.duration)}</span>
                  <button
                    className={`mobile-track-fav${t.isFavourite ? " active" : ""}`}
                    aria-label={t.isFavourite ? "Remove favourite" : "Add favourite"}
                    onClick={(e) => {
                      e.stopPropagation();
                      toggleTrackFav(t);
                    }}
                  >
                    {t.isFavourite ? <IconStarFilled /> : <IconStarEmpty />}
                  </button>
                  <div className="mobile-menu-wrap" data-menu-wrap>
                    <button
                      className="mobile-dots"
                      onClick={(e) => {
                        e.stopPropagation();
                        setOpenMenuKey((prev) => (prev === t.ratingKey ? null : t.ratingKey));
                      }}
                      aria-label="More actions"
                    >
                      <IconMoreDots size={18} />
                    </button>
                    {openMenuKey === t.ratingKey && (
                      <div className="mobile-dropdown">
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            queueAction(insertNext, [t]);
                          }}
                        >
                          Play Next
                        </button>
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            queueAction(appendToQueue, [t]);
                          }}
                        >
                          Add to Queue
                        </button>
                        <TrackDownloadMenuItem
                          ratingKey={t.ratingKey}
                          onDone={() => setOpenMenuKey(null)}
                        />
                      </div>
                    )}
                  </div>
                </div>
              </li>
            );
          })}
        </ul>

        {(album.studio || codec) && (
          <div className="mobile-detail-footer">
            {album.studio && <span>{album.studio}</span>}
            {codec && <span>{codec}</span>}
          </div>
        )}
      </div>
    </div>
  );
}
