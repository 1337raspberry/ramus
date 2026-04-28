// Typed wrappers around Tauri invoke() for every IPC command.

import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import type {
  AcknowledgementsText,
  Album,
  ArtistInfo,
  CacheStats,
  ConnectionStatusPayload,
  DownloadsOverview,
  GenreTreeResponse,
  LibrarySection,
  LyricsResult,
  PlexServer,
  SearchDownloadEstimate,
  SearchResult,
  Settings,
  SpectrumState,
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

export const connectToDiscovered = (machineIdentifier: string) =>
  invoke<{ uri: string; local: boolean; isHttp: boolean }>("connect_to_discovered", {
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

export const getAlbumsForGenreNames = (genres: string[]) =>
  invoke<Album[]>("get_albums_for_genre_names", { genres });

export const getAllAlbums = () => invoke<Album[]>("get_all_albums");

export const getFavouriteTracks = () => invoke<Track[]>("get_favourite_tracks");

export const getAlbumsForArtist = (sourceId: string) =>
  invoke<Album[]>("get_albums_for_artist", { sourceId });

export const getAlbumsForArtistName = (name: string) =>
  invoke<Album[]>("get_albums_for_artist_name", { name });

export const getAlbumsForYear = (year: number) => invoke<Album[]>("get_albums_for_year", { year });

export const getTracksForAlbum = (sourceId: string) =>
  invoke<Track[]>("get_tracks_for_album", { sourceId });

export const getTrack = (sourceId: string) => invoke<Track | null>("get_track", { sourceId });

export const getAllArtists = () => invoke<ArtistInfo[]>("get_all_artists");

export interface AlbumFilterParamsIPC {
  unplayed: boolean;
  favouriteAlbums: boolean;
  favouriteTracks: boolean;
  yearMin: number | null;
  yearMax: number | null;
  countries: string[];
  genres: string[];
  collection: string | null;
}

export const getFilteredGenreTree = (filters: AlbumFilterParamsIPC) =>
  invoke<GenreTreeResponse>("get_filtered_genre_tree", { filters });

export const getFilteredRandomAlbum = (filters: AlbumFilterParamsIPC) =>
  invoke<Album | null>("get_filtered_random_album", { filters });

export const toggleAlbumFavourite = (sourceId: string, favourite: boolean) =>
  invoke<void>("toggle_album_favourite", { sourceId, favourite });

export const toggleTrackFavourite = (sourceId: string, favourite: boolean) =>
  invoke<void>("toggle_track_favourite", { sourceId, favourite });

export const getAlbumGenres = (sourceId: string) =>
  invoke<string[]>("get_album_genres", { sourceId });

export const getAlbum = (sourceId: string) => invoke<Album | null>("get_album", { sourceId });

export const getRandomAlbum = () => invoke<Album | null>("get_random_album");

/**
 * Canonical album-art size tiers. Every surface that loads album art must
 * pick one of these; adding a fourth tier adds a fourth cached copy per
 * album on disk.
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

export const getDistinctCountries = () => invoke<string[]>("get_distinct_countries");

export const getAllCollectionNames = () => invoke<string[]>("get_all_collection_names");

export const getGenreSuggestions = (query: string, limit = 200) =>
  invoke<string[]>("get_genre_suggestions", { query, limit });

export const expandGenreToLibraryTags = (genre: string) =>
  invoke<string[]>("expand_genre_to_library_tags", { genre });

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

export interface EqConfig {
  frequencies: number[];
  minGain: number;
  maxGain: number;
}

export const getEqConfig = () => invoke<EqConfig>("get_eq_config");

export const fetchLyrics = (ratingKey: string) =>
  invoke<LyricsResult | null>("fetch_lyrics", { ratingKey });

export const getWaveform = (ratingKey: string) =>
  invoke<number[] | null>("get_waveform", { ratingKey });

// Push the current UI accent colour (0–255 sRGB) down to the OS media
// widget. Android tints the lock-screen notification with it; desktop
// + iOS accept the call and no-op.
export const setMediaAccent = (r: number, g: number, b: number) =>
  invoke<void>("set_media_accent", { r, g, b });

// Focus-mode spectrogram. Returns "analysing", { ready: … }, or
// { unavailable: { reason } }. The backend never blocks on analysis;
// callers should listen for `spectrum-ready` before re-invoking.
export const getSpectrum = (ratingKey: string) =>
  invoke<SpectrumState>("get_spectrum", { ratingKey });

// --- Search ---

export const search = (query: string, limit?: number) =>
  invoke<SearchResult[]>("search", { query, limit });

export const searchAlbumsForGrid = (query: string) =>
  invoke<Album[]>("search_albums_for_grid", { query });

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
  invoke<{
    entryCount: number;
    totalSizeBytes: number;
    pinnedCount: number;
    pinnedSizeBytes: number;
  }>("get_image_cache_stats");

export const clearAudioCache = () => invoke<void>("clear_audio_cache");

export const getAudioCacheStats = () =>
  invoke<{ entryCount: number; totalSizeBytes: number }>("get_audio_cache_stats");

// --- Debug ---

export interface DebugInfo {
  source: string;
  resolvedUrl: string | null;
  serverUrl: string | null;
  isRemote: boolean;
  playbackMode: string;
  isLoading: boolean;
  queueLen: number;
  queueIndex: number;
  lookaheadDepth: number;
  cachedInLookahead: number;
  totalInLookahead: number;
  codec: string | null;
  bitrate: number | null;
  fileSizeBytes: number | null;
}

export const getDebugInfo = () => invoke<DebugInfo>("get_debug_info");

// --- Acknowledgements / licenses ---

export const getAcknowledgementsText = () =>
  invoke<AcknowledgementsText>("get_acknowledgements_text");

// --- Platform ---

export const dismissKeyboard = () => invoke<void>("dismiss_keyboard");

export const showNativeSearchBar = (initialQuery: string) =>
  invoke<void>("show_native_search_bar", { initialQuery });

export const hideNativeSearchBar = () => invoke<void>("hide_native_search_bar");

// --- Downloads ---

export const downloadTrack = (ratingKey: string) => invoke<void>("download_track", { ratingKey });

export const downloadAlbum = (albumRatingKey: string) =>
  invoke<number>("download_album", { albumRatingKey });

export const downloadAllStarredTracks = () => invoke<number>("download_all_starred_tracks");

export const downloadAllStarredAlbums = () => invoke<number>("download_all_starred_albums");

export const cancelDownload = (ratingKey: string) => invoke<void>("cancel_download", { ratingKey });

export const cancelAllDownloads = () => invoke<void>("cancel_all_downloads");

export const removeDownload = (ratingKey: string) => invoke<void>("remove_download", { ratingKey });

export const removeAlbumDownloads = (albumRatingKey: string) =>
  invoke<number>("remove_album_downloads", { albumRatingKey });

export const removeAllDownloads = () => invoke<number>("remove_all_downloads");

export const getDownloadsOverview = () => invoke<DownloadsOverview>("get_downloads_overview");

export const estimateStarredTracksSize = () => invoke<number>("estimate_starred_tracks_size");

export const estimateStarredAlbumsSize = () => invoke<number>("estimate_starred_albums_size");

export const downloadSearchResults = (query: string) =>
  invoke<number>("download_search_results", { query });

export const estimateSearchSize = (query: string) =>
  invoke<SearchDownloadEstimate>("estimate_search_size", { query });

// --- Connection status / offline mode ---

export const getConnectionStatus = () => invoke<ConnectionStatusPayload>("get_connection_status");
