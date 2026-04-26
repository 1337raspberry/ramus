// TypeScript mirrors of ramus-core Rust models (camelCase-serialized).

export interface GenreNode {
  id: string;
  name: string;
  shortSummary: string | null;
  children: GenreNode[] | null;
  albumCount: number;
  deduplicatedTotalCount: number;
}

export interface GenreTreeResponse {
  tree: GenreNode[];
  totalAlbumCount: number;
}

export interface Album {
  ratingKey: string;
  title: string;
  artistName: string;
  year: number | null;
  thumb: string | null;
  genres: string[];
  collections: string[];
  isFavourite: boolean;
  studio: string | null;
  addedAt: number | null;
  lastViewedAt: number | null;
  viewCount: number | null;
  format: string | null;
  artistCountry: string | null;
}

export interface Track {
  ratingKey: string;
  title: string;
  artistName: string;
  trackArtist: string | null;
  albumTitle: string;
  albumKey: string | null;
  index: number | null;
  duration: number;
  codec: string | null;
  partKey: string | null;
  thumb: string | null;
  isFavourite: boolean;
  bitrate: number | null;
  discNumber: number | null;
  /// Bytes. Populated at sync time from the Plex Part response.
  fileSizeBytes: number | null;
  ratingCount: number | null;
}

export interface ArtistInfo {
  id: number;
  name: string;
  sourceId: string;
  artUrl: string | null;
  country: string | null;
}

export interface PlexServer {
  machineIdentifier: string;
  name: string;
  owned: boolean;
  connections: PlexServerConnection[];
}

export interface PlexServerConnection {
  uri: string;
  local: boolean;
  relay: boolean;
  protocol: string;
}

export interface LibrarySection {
  key: string;
  title: string;
  sectionType: string;
}

export interface SearchResult {
  id: string;
  kind: "album" | "track";
  albumSourceId: string;
  albumTitle: string;
  artistName: string;
  year: number | null;
  albumArtPath: string | null;
  trackSourceId: string | null;
  trackTitle: string | null;
  trackArtist: string | null;
  isFavourite: boolean;
  score: number;
}

export interface SavedSearch {
  id: string;
  name: string;
  query: string;
}

export const MAX_SAVED_SEARCHES = 20;

export interface Settings {
  playbackMode: "directPlay" | "transcodeLosslessRemote" | "transcodeLossless";
  lookaheadDepth: number;
  audioCacheLimitBytes: number;
  imageCacheLimitBytes: number;
  syncIntervalHours: number;
  genreSource: "open" | "custom";
  libraryPadding: number;
  refuseHttp: boolean;
  lastSyncTimeSecs: number;
  disableSpectrum: boolean;
  flatGenres: boolean;
  genreFuzzyThreshold: number;
  eqEnabled: boolean;
  eqBands: number[];
  savedSearches: SavedSearch[];
  offlineMode: boolean;
  popularityDisplay: "off" | "hot" | "chart";
}

export interface CacheStats {
  artistCount: number;
  albumCount: number;
  trackCount: number;
  genreCount: number;
}

export interface LyricLine {
  id: number;
  timestamp: number | null;
  text: string;
}

export interface LyricsResult {
  lines: LyricLine[];
  isSynced: boolean;
  source: "plex" | "lrclib";
}

export interface SyncProgress {
  phase: "artists" | "albums" | "tracks" | "deepGenres" | "done";
  current: number;
  total: number;
  detail: string;
}

export interface PlaybackStatePayload {
  status: string;
  currentTrack: Track | null;
  queueIndex: number;
}

export interface PlaybackPositionPayload {
  position: number;
  duration: number;
}

export interface AccentColorPayload {
  r: number;
  g: number;
  b: number;
}

// --- Focus-mode FFT spectrogram ---
//
// Shape mirrors ramus-core's `SpectrumFrames` (serde externally-tagged).
// `SpectrumState` is returned from the `get_spectrum` command and drives
// FocusVisualizer's bar heights.

export interface SpectrumFrames {
  /// Milliseconds between adjacent frames. Index as
  /// `floor(positionMs / hopMs)` against mpv's `time-pos`.
  hopMs: number;
  /// Number of bands per frame (128 with current defaults).
  bandCount: number;
  /// FFT window size in samples; diagnostics only.
  fftSize: number;
  /// Source sample rate; diagnostics only.
  sampleRate: number;
  /// `bandCount * totalFrames` bytes, row-major, u8 quantised 0..255.
  /// JSON IPC delivers `Vec<u8>` as a plain number array; convert to
  /// `Uint8Array` on receive.
  frames: number[] | Uint8Array;
}

/// Mirrors ramus-core's `SpectrumState` enum (externally tagged).
/// Keep in sync with `ramus-core/src/playback/spectrum.rs`.
export type SpectrumState =
  | "analysing"
  | { ready: SpectrumFrames }
  | { unavailable: { reason: string } };

/// Exhaustive-match narrowing helper for `SpectrumState`.
export function spectrumKind(state: SpectrumState): "analysing" | "ready" | "unavailable" {
  if (state === "analysing") return "analysing";
  if ("ready" in state) return "ready";
  return "unavailable";
}

export interface SpectrumReadyPayload {
  ratingKey: string;
}

export interface UltraBlurColors {
  topLeft: string;
  topRight: string;
  bottomRight: string;
  bottomLeft: string;
}

export interface AcknowledgementsText {
  mitLicense: string;
  notice: string;
  thirdParty: string;
  lgpl: string;
  mpl: string;
}

// --- Downloads ---

export type DownloadPhase = "queued" | "downloading" | "done" | "failed";

export interface DownloadProgressPayload {
  ratingKey: string;
  albumRatingKey: string;
  title: string;
  artistName: string;
  albumTitle: string;
  thumb: string | null;
  phase: DownloadPhase;
  bytesWritten: number;
  totalBytes: number | null;
  error: string | null;
}

export interface InProgressDownload {
  ratingKey: string;
  albumRatingKey: string;
  title: string;
  artistName: string;
  albumTitle: string;
  thumb: string | null;
  bytesWritten: number;
  totalBytes: number | null;
}

export interface DownloadedAlbumSummary {
  ratingKey: string;
  title: string;
  artistName: string;
  thumb: string | null;
  downloaded: number;
  total: number;
  sizeBytes: number;
}

export interface DownloadedTrackSummary {
  ratingKey: string;
  albumRatingKey: string;
  title: string;
  artistName: string;
  albumTitle: string;
  thumb: string | null;
  sizeBytes: number;
  codec: string;
}

export interface DownloadsOverview {
  inProgress: InProgressDownload | null;
  /// Preview slice of the backend user_queue (first 64 items). The full
  /// count lives in `queueLen`.
  queue: string[];
  queueLen: number;
  totalBytes: number;
  albums: DownloadedAlbumSummary[];
  orphanTracks: DownloadedTrackSummary[];
  /// Every downloaded track's rating key, for O(1) lookups from the
  /// "is this track playable offline" fade check.
  downloadedRatingKeys: string[];
}

export interface SearchDownloadEstimate {
  totalBytes: number;
  trackCount: number;
  albumCount: number;
}

// --- Connection / offline mode ---

export interface ConnectionStatusPayload {
  online: boolean;
  offlineModeManual: boolean;
  effectiveOffline: boolean;
}
