// TypeScript mirrors of ramus-core Rust models (camelCase-serialized).

/**
 * Separator joining ancestor names inside a path-based genre `id`.
 * Must match `GENRE_ID_SEP` in `ramus-core/src/genre/node.rs` — the backend
 * builds these ids, the frontend splits them to reconstruct breadcrumb trails.
 * The ASCII Unit Separator (U+001F) is used because genre names can contain
 * any printable character (e.g. "Reggae / Ska / Dancehall"); a printable
 * separator would be mis-split into phantom ancestors.
 */
export const GENRE_ID_SEP = "\u001f";

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

/** One rendered piece of a genre description. Links carry `inLibrary`: a genre
 * with matching albums, or an owned artist. Genre links always drill into the
 * genre's info; artist links only navigate when in library. `inLibrary` also
 * drives styling (accent always; bold + dotted underline when in library). */
export type DescriptionSegment =
  | { kind: "text"; value: string }
  | { kind: "genreLink"; value: string; inLibrary: boolean }
  | { kind: "artistLink"; value: string; inLibrary: boolean; navName: string | null };

/**
 * Display-only metadata for a single genre, fetched on demand (e.g. when
 * inspecting a genre pill). Present only when the active tree was imported
 * with richer metadata; the bundled tree returns null from the lookup.
 * The description arrives pre-segmented: `**Genre**` and `{{Artist}}` markup
 * is already resolved into link segments. `inLibrary` reports whether this
 * genre itself has albums (drives the title's link/underline).
 */
export interface GenreMetadata {
  canonicalName: string;
  inLibrary: boolean;
  shortSummary: string | null;
  cosmeticAka: string[];
  descriptionSegments: DescriptionSegment[];
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
  hasFavouriteTrack: boolean;
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

export interface SearchArtistResult {
  sourceId: string;
  name: string;
  artUrl: string | null;
  albumCount: number;
  score: number;
}

export interface SearchAlbumResult {
  sourceId: string;
  title: string;
  artistName: string;
  year: number | null;
  artUrl: string | null;
  /** Album rating 0–10 (5-star display divides by 2). */
  rating: number | null;
  /** Display badge like "FLAC" or "MP3 320". */
  quality: string | null;
  isFavourite: boolean;
  score: number;
}

export interface SearchTrackResult {
  sourceId: string;
  title: string;
  displayArtist: string;
  albumSourceId: string;
  albumTitle: string;
  artUrl: string | null;
  /** Track user rating 0–10. */
  rating: number | null;
  isFavourite: boolean;
  score: number;
}

export interface SearchGenreResult {
  name: string;
  albumCount: number;
  score: number;
}

/** Discriminated section union — mirrors Rust's tagged SearchSection enum. */
export type SearchSection =
  | { kind: "artists"; items: SearchArtistResult[] }
  | { kind: "albums"; items: SearchAlbumResult[] }
  | { kind: "tracks"; items: SearchTrackResult[] }
  | { kind: "genres"; items: SearchGenreResult[] };

/** Sections arrive pre-ordered (strongest first); empty sections omitted. */
export interface SearchResponse {
  sections: SearchSection[];
}

import type { AlbumFilterParamsIPC } from "./filters";

export interface Bookmark {
  id: string;
  name: string;
  filters: AlbumFilterParamsIPC;
}

/// User-requested cap; backend enforces the same value via `Bookmark::validate_batch`.
export const MAX_BOOKMARKS = 50;

/**
 * Monotonic ladder — each mode transcodes at least everywhere the one before
 * it does. Every mode except `never` adapts to a connection that can't sustain
 * the stream; `whenSlowOrCellular` additionally transcodes on cellular without
 * waiting to stall first, so it's mobile-only (`isCellular` is always false on
 * desktop). The retired `remote` / `remoteOrCellular` / `cellular` values are
 * migrated on load by the Rust side.
 */
export type PlaybackMode = "never" | "whenSlow" | "whenSlowOrCellular" | "always";

export type TranscodeBitrate = "kbps320" | "kbps256" | "kbps192" | "kbps128";

export type DownloadQuality = "original" | "kbps320" | "kbps256" | "kbps192" | "kbps128";

export interface Settings {
  playbackMode: PlaybackMode;
  /** Bitrate (kbps as enum) the universal transcoder targets when transcoding. */
  transcodeBitrate: TranscodeBitrate;
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
  eqEnabled: boolean;
  eqBands: number[];
  bookmarks: Bookmark[];
  offlineMode: boolean;
  popularityDisplay: "off" | "hot" | "chart";
  /** When true, Plex `Style` tags are merged into the genre table at sync. */
  includePlexStyles: boolean;
  /** When true, country-of-origin flags render next to artist names. */
  showArtistFlags: boolean;
  /** Quality used for user-initiated downloads. Lossless tracks transcode
   *  to Ogg/Opus at the chosen bitrate; lossy tracks always direct-play. */
  downloadQuality: DownloadQuality;
  /** Background colour styling: art-derived, brand default everywhere,
   *  or pure-black backdrop (accent still follows the artwork). */
  backgroundStyle: BackgroundStyle;
}

export type BackgroundStyle = "dynamic" | "defaultColours" | "oledVoid";

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

/**
 * Honest outcome of a lyrics fetch, so the UI can distinguish "no lyrics
 * exist" from a network problem instead of showing a blanket message.
 */
export type LyricsStatus = "found" | "notFound" | "offline" | "unreachable";

export interface LyricsFetchResult {
  status: LyricsStatus;
  lyrics: LyricsResult | null;
}

export interface SyncProgress {
  phase: "artists" | "albums" | "tracks" | "deepGenres" | "done" | "error";
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

export interface PlaybackBufferingPayload {
  buffering: boolean;
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

/// A background warm landed a metadata artefact on disk — waveform sidecar
/// (`kind: "waveform"`, has ratingKey) or cached album art (`kind: "art"`,
/// has thumb). Drives retry of surfaces still showing placeholders.
export interface MetadataWarmedPayload {
  kind: "waveform" | "art";
  ratingKey: string | null;
  thumb: string | null;
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

export interface BookmarkDownloadEstimate {
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
