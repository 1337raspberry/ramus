import { useCallback, useEffect, useState } from "react";
import { useLibraryStore } from "../stores/libraryStore";
import { getArtUrl, getQueue, insertNext, appendToQueue } from "../lib/commands";
import { usePlaybackStore } from "../stores/playbackStore";
import { formatDuration, formatCodec } from "../lib/format";

export default function AlbumDetailView() {
  const album = useLibraryStore((s) => s.detailAlbum);
  const tracks = useLibraryStore((s) => s.detailTracks);
  const closeAlbumDetail = useLibraryStore((s) => s.closeAlbumDetail);
  const playAlbum = useLibraryStore((s) => s.playAlbum);
  const toggleAlbumFav = useLibraryStore((s) => s.toggleAlbumFav);
  const toggleTrackFav = useLibraryStore((s) => s.toggleTrackFav);

  const [artSrc, setArtSrc] = useState<string | null>(null);
  const [artErr, setArtErr] = useState(false);
  const [openMenuKey, setOpenMenuKey] = useState<string | null>(null);

  // Fetch art
  useEffect(() => {
    if (!album?.thumb) { setArtSrc(null); return; }
    setArtErr(false);
    setArtSrc(null);
    let cancelled = false;
    getArtUrl(album.thumb, 160)
      .then((url) => { if (!cancelled) setArtSrc(url); })
      .catch(() => { if (!cancelled) setArtErr(true); });
    return () => { cancelled = true; };
  }, [album?.thumb]);

  // Close dropdown on outside click (only when menu is open)
  useEffect(() => {
    if (!openMenuKey) return;
    const handler = (e: MouseEvent) => {
      if (!(e.target as Element).closest(".adv-dropdown, .adv-track-dots")) {
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

  // Group tracks by disc if multi-disc
  const hasMultipleDiscs = tracks.some((t) => (t.discNumber ?? 1) > 1);

  return (
    <div className="adv-root">
      {/* Header */}
      <div className="adv-header">
        <button className="adv-back" onClick={closeAlbumDetail}>
          {"\u2039"}
        </button>
        <h1 className="adv-title">{album.title}</h1>
        <button
          className={`adv-album-fav${album.isFavourite ? " active" : ""}`}
          onClick={() => toggleAlbumFav(album)}
        >
          {album.isFavourite ? "\u2605" : "\u2606"}
        </button>
      </div>

      {/* Hero */}
      <div className="adv-hero">
        <div className="adv-art-wrap">
          {artSrc && !artErr ? (
            <img className="adv-art" src={artSrc} alt={album.title} />
          ) : (
            <div className="adv-art-placeholder">{"\u266B"}</div>
          )}
        </div>
        <div className="adv-hero-info">
          <div className="adv-artist">{album.artistName}</div>
          {album.year && <div className="adv-year">{album.year}</div>}
        </div>
        <button className="adv-hero-play" onClick={handlePlayAlbum} title="Play Album">
          {"\u25B6"}
        </button>
      </div>

      {/* Track summary */}
      <div className="adv-summary">
        {tracks.length} {tracks.length === 1 ? "track" : "tracks"} &mdash;{" "}
        {Math.round(totalDuration / 60)} minutes
      </div>

      {/* Track list */}
      <div className="adv-tracks">
        {tracks.map((track, i) => {
          const showDiscHeader =
            hasMultipleDiscs &&
            (i === 0 || track.discNumber !== tracks[i - 1].discNumber);
          const hasTrackArtist =
            track.trackArtist &&
            track.trackArtist.toLowerCase() !== album.artistName.toLowerCase();
          const isMenuOpen = openMenuKey === track.ratingKey;
          const isNearBottom = i >= tracks.length - 3;

          return (
            <div key={track.ratingKey}>
              {showDiscHeader && (
                <div className="adv-disc-header">
                  Disc {track.discNumber ?? 1}
                </div>
              )}
              <div
                className="adv-track-row"
                onClick={() => playAlbum(album, i)}
              >
                <span className="adv-track-num">{track.index ?? i + 1}</span>
                <div className="adv-track-info">
                  <div className="adv-track-title">{track.title}</div>
                  {hasTrackArtist && (
                    <div className="adv-track-artist">{track.trackArtist}</div>
                  )}
                </div>
                <span className="adv-track-duration">
                  {formatDuration(track.duration)}
                </span>
                <button
                  className={`adv-track-fav${track.isFavourite ? " active" : ""}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    toggleTrackFav(track);
                  }}
                >
                  {track.isFavourite ? "\u2605" : "\u2606"}
                </button>
                <div className="adv-track-menu-wrap">
                  <button
                    className="adv-track-dots"
                    onClick={(e) => {
                      e.stopPropagation();
                      setOpenMenuKey((prev) =>
                        prev === track.ratingKey ? null : track.ratingKey
                      );
                    }}
                  >
                    {"\u22EF"}
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
                    </div>
                  )}
                </div>
              </div>
            </div>
          );
        })}
      </div>

      {/* Footer */}
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
