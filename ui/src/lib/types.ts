// TypeScript types matching ramus-core Rust models (camelCase serialized)

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
  isFavourite: boolean;
  studio: string | null;
  addedAt: number | null;
  lastViewedAt: number | null;
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
}

export interface ArtistInfo {
  id: number;
  name: string;
  sourceId: string;
  artUrl: string | null;
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

export interface PlaybackBufferingPayload {
  isBuffering: boolean;
  bufferedFraction: number;
}

export interface ConnectionChangedPayload {
  serverUrl: string;
  isLocal: boolean;
  isHttp: boolean;
}

export interface AccentColorPayload {
  r: number;
  g: number;
  b: number;
}

// --- Focus-mode FFT spectrogram ---
//
// Shape matches ramus-core's `SpectrumFrames` struct, serde-serialised
// with externally-tagged enums (the default). `SpectrumState` is
// returned from the `get_spectrum` Tauri command and is the only thing
// that drives FocusVisualizer's bar heights — there is no live audio
// meter any more.

export interface SpectrumFrames {
  /// Milliseconds between adjacent frames. Index the spectrogram as
  /// `floor(positionMs / hopMs)` using mpv's reported `time-pos`.
  hopMs: number;
  /// Number of bands per frame (128 with current defaults).
  bandCount: number;
  /// FFT window size in samples — diagnostics only.
  fftSize: number;
  /// Source sample rate — diagnostics only.
  sampleRate: number;
  /// `bandCount * totalFrames` bytes, row-major, u8 quantised 0..255.
  /// Over Tauri's JSON IPC, postcard-ish `Vec<u8>` lands as a plain
  /// number array. We convert to `Uint8Array` on receive.
  frames: number[] | Uint8Array;
}

/// Matches ramus-core's `SpectrumState` enum (externally tagged).
/// See `ramus-core/src/playback/spectrum.rs` — keep these in sync.
export type SpectrumState =
  | "analysing"
  | { ready: SpectrumFrames }
  | { unavailable: { reason: string } };

/// Helper for exhaustive-match narrowing on `SpectrumState`.
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
