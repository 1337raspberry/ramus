import { useCallback } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useLibraryStore } from "../stores/libraryStore";
import { formatCodec } from "./format";

interface Options {
  /**
   * Fired after a navigation action (artist/album/year/genre click).
   * FocusNowPlayingView passes this to exit focus mode; the compact
   * NowPlayingView omits it.
   */
  onNavigate?: () => void;
}

/**
 * Shared derived values and click handlers for Now Playing views.
 *
 * Favourite toggles route through `libraryStore` so optimistic updates
 * propagate to every slice (see CLAUDE.md). Navigation actions go through
 * `libraryStore` store methods rather than raw IPC.
 */
export function useNowPlayingActions(options?: Options) {
  const track = usePlaybackStore((s) => s.currentTrack);
  const nowPlayingAlbum = usePlaybackStore((s) => s.nowPlayingAlbum);
  const onNavigate = options?.onNavigate;

  const hasTrackArtist = !!(
    track?.trackArtist && track.trackArtist.toLowerCase() !== track.artistName.toLowerCase()
  );
  const year = nowPlayingAlbum?.year ?? null;
  const studio = nowPlayingAlbum?.studio ?? null;
  const codec = track ? formatCodec(track.codec, track.bitrate) : null;
  const albumFav = nowPlayingAlbum?.isFavourite ?? false;
  const trackFav = track?.isFavourite ?? false;

  const handleAlbumFavToggle = useCallback(() => {
    if (!nowPlayingAlbum) return;
    useLibraryStore.getState().toggleAlbumFav(nowPlayingAlbum);
  }, [nowPlayingAlbum]);

  const handleTrackFavToggle = useCallback(() => {
    if (!track) return;
    useLibraryStore.getState().toggleTrackFav(track);
  }, [track]);

  const handleArtistClick = useCallback(() => {
    if (!track) return;
    useLibraryStore.getState().loadAlbumsForArtistName(track.artistName);
    onNavigate?.();
  }, [track, onNavigate]);

  const handleAlbumClick = useCallback(() => {
    if (!nowPlayingAlbum) return;
    useLibraryStore.getState().openAlbumDetail(nowPlayingAlbum);
    onNavigate?.();
  }, [nowPlayingAlbum, onNavigate]);

  const handleYearClick = useCallback(() => {
    if (!year) return;
    useLibraryStore.getState().loadAlbumsForYear(year);
    onNavigate?.();
  }, [year, onNavigate]);

  const handleGenreClick = useCallback(
    (genre: string) => {
      useLibraryStore.getState().selectGenreByName(genre);
      onNavigate?.();
    },
    [onNavigate],
  );

  return {
    track,
    nowPlayingAlbum,
    hasTrackArtist,
    year,
    studio,
    codec,
    albumFav,
    trackFav,
    handleAlbumFavToggle,
    handleTrackFavToggle,
    handleArtistClick,
    handleAlbumClick,
    handleYearClick,
    handleGenreClick,
  };
}
