import { useCallback } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useLibraryStore } from "../stores/libraryStore";
import { formatCodec } from "./format";

interface Options {
  /**
   * Optional callback fired AFTER a navigation action (artist/album/year/genre click).
   * FocusNowPlayingView uses this to exit focus mode so the user sees the
   * updated main layout. The compact NowPlayingView omits it.
   */
  onNavigate?: () => void;
}

/**
 * Shared derived values and click handlers for Now Playing views.
 *
 * Both `NowPlayingView` (compact right-column panel) and
 * `FocusNowPlayingView` (full-screen overlay) render the same click
 * affordances for artist/album/year/genre navigation plus the two
 * favourite toggles. This hook centralises the wiring so the two views
 * don't drift: all favourite toggles route through `libraryStore` so the
 * optimistic update propagates to every relevant slice (see CLAUDE.md),
 * and navigation actions go through `libraryStore` store methods rather
 * than raw IPC commands.
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
      const store = useLibraryStore.getState();
      store.setSidebarMode("genres");
      store.loadAlbumsForGenre(genre);
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
