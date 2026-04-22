import { useCallback, useEffect, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import {
  ART_SIZE,
  getArtUrl,
  getQueue,
  insertNext,
  appendToQueue,
  getAlbumGenres,
} from "../lib/commands";
import { usePlaybackStore } from "../stores/playbackStore";
import { useDownloadsStore } from "../stores/downloadsStore";
import { useConnectionStatus } from "../lib/useConnectionStatus";
import { AlbumDownloadMenuItem, TrackDownloadMenuItem } from "./DownloadMenuItems";
import { formatDuration, formatCodec } from "../lib/format";
import { countryToFlag } from "../lib/countryFlag";
import {
  IconChevronLeft,
  IconStarFilled,
  IconStarEmpty,
  IconMusicNote,
  IconPlay,
  IconMoreDots,
} from "./Icons";
import FlowLayout from "./FlowLayout";

export default function AlbumDetailView() {
  const album = useLibraryStore((s) => s.detailAlbum);
  const tracks = useLibraryStore((s) => s.detailTracks);
  const closeAlbumDetail = useLibraryStore((s) => s.closeAlbumDetail);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const toggleAlbumFav = useLibraryStore((s) => s.toggleAlbumFav);
  const toggleTrackFav = useLibraryStore((s) => s.toggleTrackFav);
  const loadAlbumsForArtistName = useLibraryStore((s) => s.loadAlbumsForArtistName);
  const loadAlbumsForYear = useLibraryStore((s) => s.loadAlbumsForYear);
  const selectGenreByName = useLibraryStore((s) => s.selectGenreByName);

  const handleGenreClick = useCallback(
    (genre: string) => {
      closeAlbumDetail();
      selectGenreByName(genre);
    },
    [closeAlbumDetail, selectGenreByName],
  );

  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artErr, setArtErr] = useState(false);
  const [openMenuKey, setOpenMenuKey] = useState<string | null>(null);
  const [genres, setGenres] = useState<string[]>([]);

  useEffect(() => {
    if (!album?.thumb) {
      setArtSrc(null);
      return;
    }
    setArtErr(false);
    setArtSrc(null);
    let cancelled = false;
    getArtUrl(album.thumb, ART_SIZE.MEDIUM)
      .then((url) => {
        if (!cancelled) setArtSrc(url);
      })
      .catch(() => {
        if (!cancelled) setArtErr(true);
      });
    return () => {
      cancelled = true;
    };
  }, [album?.thumb]);

  useEffect(() => {
    if (!album) {
      setGenres([]);
      return;
    }
    // Use album.genres when present; otherwise fetch via IPC. The DB
    // layer's `map_album_rows` leaves genres empty by default.
    if (album.genres.length) {
      setGenres(album.genres);
      return;
    }
    let cancelled = false;
    getAlbumGenres(album.ratingKey)
      .then((g) => {
        if (!cancelled) setGenres(g);
      })
      .catch(() => {
        if (!cancelled) setGenres([]);
      });
    return () => {
      cancelled = true;
    };
  }, [album]);

  // Close dropdown on outside click.
  useEffect(() => {
    if (!openMenuKey) return;
    const handler = (e: MouseEvent) => {
      if (!(e.target as Element).closest(".adv-dropdown, .adv-track-dots, .adv-hero-dots")) {
        setOpenMenuKey(null);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [openMenuKey]);

  const handlePlayAlbum = useCallback(() => {
    if (album) playAlbum(album);
  }, [album, playAlbum]);

  if (!album) return null;

  const totalDuration = tracks.reduce((sum, t) => sum + t.duration, 0);
  const codec = tracks.length ? formatCodec(tracks[0].codec, tracks[0].bitrate) : null;

  const hasMultipleDiscs = tracks.some((t) => (t.discNumber ?? 1) > 1);

  // Offline-mode gate: tracks the user doesn't have locally can't be
  // played. Subscribe to the relevant selectors up here instead of inside
  // the render map so the component only re-subscribes when the sets
  // actually change.
  const { effectiveOffline } = useConnectionStatus();
  const downloadedIds = useDownloadsStore((s) => s.downloadedTrackIds);

  return (
    <div className="adv-root">
      <div className="adv-header">
        <button className="adv-back" onClick={closeAlbumDetail}>
          <IconChevronLeft />
        </button>
        <h1 className="adv-title">{album.title}</h1>
        <button
          className={`adv-album-fav${album.isFavourite ? " active" : ""}`}
          onClick={() => toggleAlbumFav(album)}
        >
          {album.isFavourite ? <IconStarFilled /> : <IconStarEmpty />}
        </button>
      </div>

      <div className="adv-hero">
        <div className="adv-art-wrap">
          {artSrc && !artErr ? (
            <img className="adv-art" src={artSrc} alt={album.title} />
          ) : (
            <div className="adv-art-placeholder">
              <IconMusicNote />
            </div>
          )}
        </div>
        <div className="adv-hero-info">
          <div
            className="adv-artist adv-link"
            onClick={() => loadAlbumsForArtistName(album.artistName)}
          >
            {album.artistName}
            {(() => {
              const flag = album.artistCountry ? countryToFlag(album.artistCountry) : null;
              return flag ? (
                <span className="adv-country-flag" title={album.artistCountry!}>
                  {flag}
                </span>
              ) : null;
            })()}
          </div>
          {album.year && (
            <div className="adv-year adv-link" onClick={() => loadAlbumsForYear(album.year!)}>
              {album.year}
            </div>
          )}
          {genres.length > 0 && (
            <div className="adv-genres">
              <FlowLayout genres={genres} onGenreClick={handleGenreClick} />
            </div>
          )}
          {album.format && album.format !== "Album" && (
            <span className="adv-format-pill">{album.format}</span>
          )}
        </div>
        <button className="adv-hero-play" onClick={handlePlayAlbum} title="Play Album">
          <IconPlay />
        </button>
        <div className="adv-hero-menu-wrap">
          <button
            className="adv-hero-dots"
            onClick={() => setOpenMenuKey((prev) => (prev === "__album__" ? null : "__album__"))}
            title="More actions"
          >
            <IconMoreDots />
          </button>
          {openMenuKey === "__album__" && (
            <div className="adv-dropdown">
              <button
                onClick={() => {
                  insertNext(tracks)
                    .then(() => getQueue())
                    .then((q) => usePlaybackStore.setState({ queue: q }))
                    .catch(() => {});
                  setOpenMenuKey(null);
                }}
              >
                Play Next
              </button>
              <button
                onClick={() => {
                  appendToQueue(tracks)
                    .then(() => getQueue())
                    .then((q) => usePlaybackStore.setState({ queue: q }))
                    .catch(() => {});
                  setOpenMenuKey(null);
                }}
              >
                Add to Queue
              </button>
              <AlbumDownloadMenuItem
                albumRatingKey={album.ratingKey}
                onDone={() => setOpenMenuKey(null)}
              />
            </div>
          )}
        </div>
      </div>

      <div className="adv-summary">
        {tracks.length} {tracks.length === 1 ? "track" : "tracks"} &mdash;{" "}
        {Math.round(totalDuration / 60)} minutes
      </div>

      <div className="adv-tracks">
        {tracks.map((track, i) => {
          const showDiscHeader =
            hasMultipleDiscs && (i === 0 || track.discNumber !== tracks[i - 1].discNumber);
          const hasTrackArtist =
            track.trackArtist && track.trackArtist.toLowerCase() !== album.artistName.toLowerCase();
          const isMenuOpen = openMenuKey === track.ratingKey;
          const isNearBottom = i >= tracks.length - 3;
          const unavailable = effectiveOffline && !downloadedIds.has(track.ratingKey);

          return (
            <div key={track.ratingKey}>
              {showDiscHeader && (
                <div className="adv-disc-header">Disc {track.discNumber ?? 1}</div>
              )}
              <div
                className={`adv-track-row${unavailable ? " adv-track-row-unavailable" : ""}`}
                onClick={() => {
                  if (unavailable) return;
                  playAlbum(album, i);
                }}
                title={unavailable ? "Not downloaded — unavailable offline" : undefined}
              >
                <span className="adv-track-num">{track.index ?? i + 1}</span>
                <div className="adv-track-info">
                  <div className="adv-track-title">{track.title}</div>
                  {hasTrackArtist && <div className="adv-track-artist">{track.trackArtist}</div>}
                </div>
                <span className="adv-track-duration">{formatDuration(track.duration)}</span>
                <button
                  className={`adv-track-fav${track.isFavourite ? " active" : ""}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleTrackFav(track);
                  }}
                >
                  {track.isFavourite ? <IconStarFilled /> : <IconStarEmpty />}
                </button>
                <div className="adv-track-menu-wrap">
                  <button
                    className="adv-track-dots"
                    onClick={(e) => {
                      e.stopPropagation();
                      setOpenMenuKey((prev) => (prev === track.ratingKey ? null : track.ratingKey));
                    }}
                  >
                    <IconMoreDots />
                  </button>
                  {isMenuOpen && (
                    <div className={`adv-dropdown${isNearBottom ? " up" : ""}`}>
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          insertNext([track])
                            .then(() => getQueue())
                            .then((q) => usePlaybackStore.setState({ queue: q }))
                            .catch(() => {});
                          setOpenMenuKey(null);
                        }}
                      >
                        Play Next
                      </button>
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          appendToQueue([track])
                            .then(() => getQueue())
                            .then((q) => usePlaybackStore.setState({ queue: q }))
                            .catch(() => {});
                          setOpenMenuKey(null);
                        }}
                      >
                        Add to Queue
                      </button>
                      <TrackDownloadMenuItem
                        ratingKey={track.ratingKey}
                        onDone={() => setOpenMenuKey(null)}
                      />
                    </div>
                  )}
                </div>
              </div>
            </div>
          );
        })}
      </div>

      <div className="adv-footer">
        <span className="adv-footer-left">
          {album.studio ?? ""}
          {album.studio && album.year ? " \u2013 " : ""}
          {album.year ?? ""}
        </span>
        {codec && <span className="adv-footer-right">{codec}</span>}
      </div>
    </div>
  );
}
