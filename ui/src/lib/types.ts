// TypeScript types matching ramus-core Rust models (camelCase serialized)

export interface GenreNode {
  id: string;
  name: string;
  shortSummary: string | null;
  children: GenreNode[] | null;
  albumCount: number;
  deduplicatedTotalCount: number;
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
  accessToken: string;
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
  score: number;
}

export interface Settings {
  playbackMode: "directPlay" | "transcodeLosslessRemote" | "transcodeLossless";
  lookaheadDepth: number;
  audioCacheLimitBytes: number;
  syncIntervalHours: number;
  genreSource: "open" | "custom";
  libraryPadding: number;
  showTaglines: boolean;
  refuseHttp: boolean;
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
