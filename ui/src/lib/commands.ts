// Typed wrappers around Tauri invoke() for every IPC command.

import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import type {
  AcknowledgementsText,
  Album,
  ArtistInfo,
  CacheStats,
  ConnectionStatusPayload,
  DownloadsOverview,
  GenreMetadata,
  GenreTreeResponse,
  LibrarySection,
  LyricsFetchResult,
  PlexServer,
  BookmarkDownloadEstimate,
  SearchResponse,
  Settings,
  SpectrumState,
  Track,
  UltraBlurColors,
} from "./types";
import type { AlbumFilterParamsIPC } from "./filters";
import type { VibrantPalette } from "./vibrantColor";

export type { AlbumFilterParamsIPC } from "./filters";

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

export const getFilteredGenreTree = (filters: AlbumFilterParamsIPC) =>
  invoke<GenreTreeResponse>("get_filtered_genre_tree", { filters });

/**
 * Random album matching `filters`, skipping `exclude` (rating keys of the
 * most recently suggested albums, newest first). The backend trims the
 * exclusion list against the post-filter pool, so passing more than a small
 * pool can hold is safe — it can never starve the draw.
 */
export const getFilteredRandomAlbum = (filters: AlbumFilterParamsIPC, exclude: string[] = []) =>
  invoke<Album | null>("get_filtered_random_album", { filters, exclude });

export const toggleAlbumFavourite = (sourceId: string, favourite: boolean) =>
  invoke<void>("toggle_album_favourite", { sourceId, favourite });

export const toggleTrackFavourite = (sourceId: string, favourite: boolean) =>
  invoke<void>("toggle_track_favourite", { sourceId, favourite });

export const getAlbumGenres = (sourceId: string) =>
  invoke<string[]>("get_album_genres", { sourceId });

export const getAlbum = (sourceId: string) => invoke<Album | null>("get_album", { sourceId });

/** Unfiltered counterpart of `getFilteredRandomAlbum`; same exclusion rules. */
export const getRandomAlbum = (exclude: string[] = []) =>
  invoke<Album | null>("get_random_album", { exclude });

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

// Coalesce concurrent art lookups for the same (thumb, size). Without this,
// every queue row / grid tile that mounts with the same artwork fires its
// own IPC + Plex fetch — and Plex's per-client concurrent cap serialises
// them, so identical images appear one-by-one on slow links.
const inFlightArt = new Map<string, Promise<string>>();

// Fixed-concurrency limiter for distinct art fetches. A single screen can
// mount hundreds of thumbnails at once (e.g. the Up Next list after a
// shuffle-all over a large favourites set), each firing its own IPC + Plex
// request. The images are tiny so total cost is trivial, but slamming the
// bridge and server with the whole burst simultaneously is needlessly
// greedy. This funnels them through a fixed number of slots; the rest wait
// in line and drain as slots free up. Coalesced duplicates (above) never
// reach here, so only genuinely distinct fetches consume a slot.
const ART_MAX_CONCURRENT = 6;
let artInFlight = 0;
const artWaiters: Array<() => void> = [];

function acquireArtSlot(): Promise<void> {
  if (artInFlight < ART_MAX_CONCURRENT) {
    artInFlight++;
    return Promise.resolve();
  }
  return new Promise((resolve) => artWaiters.push(resolve));
}

function releaseArtSlot(): void {
  const next = artWaiters.shift();
  // Hand the slot straight to the next waiter (count stays put); only
  // decrement when no one is waiting.
  if (next) next();
  else artInFlight--;
}

export const getArtUrl = (thumb: string, size?: number): Promise<string> => {
  const key = `${size ?? 300}::${thumb}`;
  const existing = inFlightArt.get(key);
  if (existing) return existing;
  const pending = (async () => {
    await acquireArtSlot();
    try {
      const filePath = await invoke<string>("get_art_url", { thumb, size });
      return convertFileSrc(filePath);
    } finally {
      // Drop the coalescing entry BEFORE freeing the slot. Releasing the
      // slot can hand it straight to a queued waiter, so the map must
      // already be clean — otherwise a same-key call racing in at that
      // instant could latch onto this settled (possibly post-flush stale)
      // promise instead of starting a fresh fetch.
      inFlightArt.delete(key);
      releaseArtSlot();
    }
  })();
  inFlightArt.set(key, pending);
  return pending;
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
  invoke<LyricsFetchResult>("fetch_lyrics", { ratingKey });

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

export const search = (query: string, sectionLimit?: number) =>
  invoke<SearchResponse>("search", { query, sectionLimit });

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

/** Import a richer genre tree from JSON; returns the total genre count across all depths. */
export const importCustomGenresJson = (text: string) =>
  invoke<number>("import_custom_genres_json", { text });

/** Display metadata for a genre, or null when the active tree carries none. */
export const getGenreMetadata = (name: string) =>
  invoke<GenreMetadata | null>("get_genre_metadata", { name });

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

export type DebugPhase = "stopped" | "paused" | "opening" | "buffering" | "playing" | "stalled";

export interface DebugInfo {
  source: string;
  resolvedUrl: string | null;
  serverUrl: string | null;
  isRemote: boolean;
  /** Platform network monitor reports a cellular path; always false on desktop. */
  isCellular: boolean;
  playbackMode: string;
  queueLen: number;
  queueIndex: number;
  lookaheadDepth: number;
  cachedInLookahead: number;
  cachedInLookaheadTranscoded: number;
  cachedInLookaheadDirect: number;
  totalInLookahead: number;
  codec: string | null;
  bitrate: number | null;
  fileSizeBytes: number | null;
  phase: DebugPhase;
  /**
   * The link is up and delivering, but too slowly to sustain this stream
   * (repeated rebuffering). Distinct from `phase: "stalled"`, which a dead
   * socket also produces.
   */
  starving: boolean;
  /** Bitrate forced by the adaptive layer, or null under the user's policy. */
  degradedToKbps: number | null;
  /**
   * mpv's raw `demuxer-cache-time` — seconds buffered ahead. Climbing during
   * a stall means bytes are still arriving; frozen means a dead socket.
   */
  demuxerCacheTime: number | null;
  /** Completed rebuffer episodes inside the starvation window. */
  starvationEpisodes: number;
  /** Seconds since the last `time-pos` update, or `null` if none yet. */
  secondsSincePositionUpdate: number | null;
  /** Seconds since the current track was loaded. */
  secondsSinceLoad: number | null;
  /** Last unrecoverable mpv END_FILE error (already URL-redacted). */
  lastLoadError: string | null;
}

export const getDebugInfo = () => invoke<DebugInfo>("get_debug_info");

/**
 * Ask the backend to re-emit the authoritative playback + connection
 * snapshot (and re-evaluate the connection if the app woke up offline or
 * with playback interrupted). Fired on foreground transitions — a
 * suspended webview may have dropped every event that fired while the OS
 * had the app asleep, and the stores are pure event replay.
 */
export const foregroundResync = () => invoke<void>("foreground_resync");

// --- Acknowledgements / licenses ---

export const getAcknowledgementsText = () =>
  invoke<AcknowledgementsText>("get_acknowledgements_text");

export const openExternalUrl = (url: string) => invoke<void>("open_external_url", { url });

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

export const downloadBookmark = (filters: AlbumFilterParamsIPC) =>
  invoke<number>("download_bookmark", { filters });

export const estimateBookmark = (filters: AlbumFilterParamsIPC) =>
  invoke<BookmarkDownloadEstimate>("estimate_bookmark", { filters });

// --- Connection status / offline mode ---

export const getConnectionStatus = () => invoke<ConnectionStatusPayload>("get_connection_status");
