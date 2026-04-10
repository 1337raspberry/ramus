// Typed wrappers around Tauri invoke() for all commands.

import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import type {
  Album,
  ArtistInfo,
  CacheStats,
  GenreTreeResponse,
  LibrarySection,
  LyricsResult,
  PlexServer,
  SearchResult,
  Settings,
  Track,
  UltraBlurColors,
} from "./types";
import type { VibrantPalette } from "./vibrantColor";

// --- Auth ---

export const startOauth = () => invoke<string>("start_oauth");

export const pollOauth = (pinId: number) => invoke<boolean>("poll_oauth", { pinId });

export const discoverServers = () => invoke<PlexServer[]>("discover_servers");

export const testServer = (machineIdentifier: string) =>
  invoke<{ connected: boolean; uri?: string; local?: boolean; isHttp?: boolean }>("test_server", {
    machineIdentifier,
  });

export const connectManualUrl = (url: string) => invoke<boolean>("connect_manual_url", { url });

export const findMusicLibraries = () => invoke<LibrarySection[]>("find_music_libraries");

export const finalizeOnboarding = (
  machineIdentifier: string,
  libraryKey: string,
  serverUrl: string,
) => invoke<void>("finalize_onboarding", { machineIdentifier, libraryKey, serverUrl });

export const isAuthenticated = () => invoke<boolean>("is_authenticated");

export const logout = () => invoke<void>("logout");

// --- Library ---

export const getGenreTree = () => invoke<GenreTreeResponse>("get_genre_tree");

export const getAlbumsForGenre = (genre: string) =>
  invoke<Album[]>("get_albums_for_genre", { genre });

export const getAllAlbums = () => invoke<Album[]>("get_all_albums");

export const getFavouriteAlbums = () => invoke<Album[]>("get_favourite_albums");

export const getFavouriteTracks = () => invoke<Track[]>("get_favourite_tracks");

export const getAlbumsForArtist = (sourceId: string) =>
  invoke<Album[]>("get_albums_for_artist", { sourceId });

export const getAlbumsForArtistName = (name: string) =>
  invoke<Album[]>("get_albums_for_artist_name", { name });

export const getAlbumsForYear = (year: number) => invoke<Album[]>("get_albums_for_year", { year });

export const getTracksForAlbum = (sourceId: string) =>
  invoke<Track[]>("get_tracks_for_album", { sourceId });

export const getAllArtists = () => invoke<ArtistInfo[]>("get_all_artists");

export const getFavouriteGenreTree = () => invoke<GenreTreeResponse>("get_favourite_genre_tree");

export const toggleAlbumFavourite = (sourceId: string, favourite: boolean) =>
  invoke<void>("toggle_album_favourite", { sourceId, favourite });

export const toggleTrackFavourite = (sourceId: string, favourite: boolean) =>
  invoke<void>("toggle_track_favourite", { sourceId, favourite });

export const getAlbumGenres = (sourceId: string) =>
  invoke<string[]>("get_album_genres", { sourceId });

export const getAlbum = (sourceId: string) => invoke<Album | null>("get_album", { sourceId });

export const getRandomAlbum = () => invoke<Album | null>("get_random_album");

/**
 * Canonical album-art size tiers. Every surface that loads album art should
 * pick one of these — it keeps the per-size on-disk cache bounded to three
 * entries per album and lets different surfaces share cache hits. Adding a
 * fourth tier means every album gets a fourth cached copy, so resist the
 * temptation unless there's a genuine new size class.
 *
 * - SMALL  (72):   search result rows, queue track thumbnails
 * - MEDIUM (300):  album grid tiles, album detail header
 * - LARGE  (1200): compact Now Playing panel, focus Now Playing, suggestion view
 */
export const ART_SIZE = {
  SMALL: 72,
  MEDIUM: 300,
  LARGE: 1200,
} as const;

export const getArtUrl = async (thumb: string, size?: number): Promise<string> => {
  const filePath = await invoke<string>("get_art_url", { thumb, size });
  return convertFileSrc(filePath);
};

export const getAlbumColors = (sourceId: string) =>
  invoke<{ colors: UltraBlurColors | null; palette: VibrantPalette | null }>("get_album_colors", {
    sourceId,
  });

export const setAlbumPalette = (sourceId: string, palette: VibrantPalette) =>
  invoke<void>("set_album_palette", { sourceId, palette });

export const getCacheStats = () => invoke<CacheStats>("get_cache_stats");

// --- Playback ---

export const playTracks = (tracks: Track[], startAt: number) =>
  invoke<void>("play_tracks", { tracks, startAt });

export const togglePlayPause = () => invoke<void>("toggle_play_pause");

export const nextTrack = () => invoke<void>("next_track");

export const previousTrack = () => invoke<void>("previous_track");

export const seek = (position: number) => invoke<void>("seek", { position });

export const setVolume = (volume: number) => invoke<void>("set_volume", { volume });

export const getVolume = () => invoke<number>("get_volume");

export const appendToQueue = (tracks: Track[]) => invoke<void>("append_to_queue", { tracks });

export const insertNext = (tracks: Track[]) => invoke<void>("insert_next", { tracks });

export const removeFromQueue = (index: number) => invoke<void>("remove_from_queue", { index });

export const jumpToQueueIndex = (index: number) => invoke<void>("jump_to_queue_index", { index });

export const getQueue = () => invoke<Track[]>("get_queue");

export const applyEqualizer = (enabled: boolean, bands: number[]) =>
  invoke<void>("apply_equalizer", { enabled, bands });

export const fetchLyrics = (ratingKey: string) =>
  invoke<LyricsResult | null>("fetch_lyrics", { ratingKey });

export const getWaveform = (ratingKey: string) =>
  invoke<number[] | null>("get_waveform", { ratingKey });

// --- Search ---

export const search = (query: string, limit?: number) =>
  invoke<SearchResult[]>("search", { query, limit });

// --- Sync ---

export const startFullSync = () => invoke<void>("start_full_sync");

export const startIncrementalSync = () => invoke<void>("start_incremental_sync");

export const startGenreSync = () => invoke<void>("start_genre_sync");

// --- Settings ---

export const getSettings = () => invoke<Settings>("get_settings");

export const updateSettings = (settings: Settings) => invoke<void>("update_settings", { settings });

export const importCustomGenres = (text: string) =>
  invoke<string[]>("import_custom_genres", { text });

export const removeCustomGenres = () => invoke<void>("remove_custom_genres");

export const hasCustomGenres = () => invoke<boolean>("has_custom_genres");

export const flushImageCache = () => invoke<void>("flush_image_cache");

export const getImageCacheStats = () =>
  invoke<{ entryCount: number; totalSizeBytes: number }>("get_image_cache_stats");
