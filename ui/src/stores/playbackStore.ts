import { create } from "zustand";
import type {
  Album,
  LyricsResult,
  LyricsStatus,
  SpectrumState,
  Track,
  UltraBlurColors,
} from "../lib/types";
import { accentFromPalette, blurColorsFromPalette, type VibrantPalette } from "../lib/vibrantColor";
import { applyAccent } from "../lib/accent";

/**
 * Focus-mode visualiser rendering mode.
 *
 * - `"off"`  — viz is unmounted, RAF loop stops
 * - `"bars"` — 256-bar mirrored spectrum, bass centred, treble at edges
 * - `"line"` — smoothed averaged curve filled from the top edge down
 *
 * Cycled via `cycleVisualizerMode`.
 */
export type VisualizerMode = "off" | "bars" | "line";
import {
  getVolume,
  setVolume as setVolumeCmd,
  seek as seekCmd,
  fetchLyrics,
  getWaveform,
  getSpectrum,
  getQueue,
  getAlbum,
  getAlbumGenres,
  getAlbumColors,
  removeFromQueue as removeFromQueueCmd,
  jumpToQueueIndex as jumpToQueueIndexCmd,
  clearQueue as clearQueueCmd,
} from "../lib/commands";

interface PlaybackState {
  // --- Playback ---
  status: "stopped" | "playing" | "paused";
  currentTrack: Track | null;
  queueIndex: number;
  position: number;
  duration: number;
  volume: number;
  /// True while audio has stalled mid-playback (position events stopped
  /// arriving while status is "playing") — initial buffer, a mid-track
  /// network hiccup, or the reload gap during a failover resume. Drives the
  /// sweeping "scanning" indicator on the waveform. Derived on the frontend
  /// (no dedicated backend event); see `usePlaybackEvents`.
  isBuffering: boolean;

  // --- Lyrics ---
  lyrics: LyricsResult | null;
  lyricsLoading: boolean;
  /// Honest result of the last fetch — drives the panel's empty-state copy
  /// (no lyrics vs offline vs server unreachable). `null` before any fetch.
  lyricsStatus: LyricsStatus | null;
  /// Whether the lyrics panel is open. Once toggled on it stays on across
  /// track changes for the whole session (re-fetching each track) and resets
  /// to off only on app restart — there is no separate "pin" any more.
  showLyrics: boolean;

  // --- Waveform ---
  waveformLevels: number[] | null;
  // --- UltraBlur ---
  ultraBlurColors: UltraBlurColors | null;
  vibrantPalette: VibrantPalette | null;

  // --- Queue ---
  queue: Track[];
  showQueue: boolean;

  // --- Now Playing Metadata ---
  nowPlayingAlbum: Album | null;
  currentGenres: string[];

  // --- Focus mode ---
  isFocusMode: boolean;
  // Session-only; resets to `"bars"` on reload. Cycled bars → line → off.
  visualizerMode: VisualizerMode;

  // --- Focus-mode FFT spectrogram ---
  //
  // Precomputed per-track bands from symphonia + realfft in Rust.
  // Hydrated on track change and on every `spectrum-ready` event.
  // FocusVisualizer reads this via getState() inside a RAF loop to avoid
  // re-renders on every 60fps tick. Do not subscribe via a React selector.
  //
  // `null` = never fetched for the current track. `"analysing"` = backend
  // hasn't finished analysis; viz shows a placeholder. `{ ready }` = viz
  // draws bars at the current position lookup.
  spectrumState: SpectrumState | null;

  // --- Event Handlers ---
  onPlaybackState: (status: string, track: Track | null, queueIndex: number) => void;
  onPlaybackPosition: (position: number, duration: number) => void;
  setBuffering: (buffering: boolean) => void;
  /// Called on `spectrum-ready` events from Rust and on track change to
  /// hydrate from the cache. Safe to call unconditionally; it only invokes
  /// `getSpectrum` when there is a current track.
  refreshSpectrum: (forRatingKey?: string) => void;
  /// Re-fetch the current track's waveform when it's still missing — called
  /// on `metadata-warmed` (a background warm just landed the sidecar) and on
  /// connection recovery. No-op when levels are already loaded, when
  /// `forRatingKey` doesn't match the current track, or when nothing is
  /// playing.
  refreshWaveform: (forRatingKey?: string) => void;

  // --- Actions ---
  seek: (seconds: number) => void;
  seekFraction: (fraction: number) => void;
  changeVolume: (volume: number) => void;
  loadVolume: () => void;
  /// Fetch lyrics for a track and store the result + honest status. Guarded
  /// by a generation counter so a slow fetch for an outgoing track can't land
  /// on a newer one (the retry path makes this race wider than it was).
  loadLyrics: (ratingKey: string) => void;
  toggleLyrics: () => void;
  toggleQueue: () => void;
  toggleFocusMode: () => void;
  cycleVisualizerMode: () => void;
  removeQueueItem: (index: number) => void;
  jumpToIndex: (index: number) => void;
  /// Stop playback and empty the queue. Clears local state optimistically;
  /// the backend's `playback-state` emit lands on the same values.
  clearQueue: () => void;
}

function activeLineIndex(lyrics: LyricsResult, position: number): number {
  if (!lyrics.isSynced) return -1;
  const lines = lyrics.lines;
  let result = -1;
  for (let i = 0; i < lines.length; i++) {
    const ts = lines[i].timestamp;
    if (ts !== null && ts <= position) {
      result = i;
    } else if (ts !== null && ts > position) {
      break;
    }
  }
  return result;
}

export { activeLineIndex };

// Monotonic generation counter for async spectrum refreshes. In-flight
// `getSpectrum` invokes compare against the captured value and drop their
// result if the track has changed.
let spectrumGen = 0;

// Same idea for lyrics: a slow `fetchLyrics` (now with retries) for the
// outgoing track must not overwrite the incoming track's lyrics. Bumped on
// every track change; each `loadLyrics` captures it and drops a stale result.
let lyricsGen = 0;

// --- UltraBlur write gate ---
//
// `ultraBlurColors` has multiple unguarded async writers (the DB
// instant-paint fetch and the art-decode extraction across several
// components) with an intended priority: art-derived extraction beats
// server-provided colours beats palette-derived fallback. Because the
// writers race (a cached image can decode BEFORE the DB round-trip
// resolves), plain last-write-wins frequently inverts that priority and
// the coarser colours stick for the whole track. All writes therefore go
// through `applyUltraBlurColors`, which enforces:
//   1. generation: a write captured under an older generation (previous
//      track / previous suggestion) is dropped, and
//   2. rank: within a generation, a lower-priority source never
//      overwrites a higher-priority one.
let ultraBlurGen = 0;
let ultraBlurRank = -1;

const ULTRABLUR_RANK = { palette: 0, server: 1, extracted: 2 } as const;
export type UltraBlurSource = keyof typeof ULTRABLUR_RANK;

/** Current generation — capture BEFORE starting an async fetch and pass
 * back to `applyUltraBlurColors` so a stale resolution is dropped. */
export function ultraBlurColorsGen(): number {
  return ultraBlurGen;
}

/** Open a new generation (track change / suggestion change): any
 * in-flight writes captured under the old generation become no-ops. */
export function resetUltraBlurGate(): void {
  ultraBlurGen += 1;
  ultraBlurRank = -1;
}

export function applyUltraBlurColors(
  colors: UltraBlurColors,
  source: UltraBlurSource,
  gen: number = ultraBlurGen,
): void {
  if (gen !== ultraBlurGen) return;
  const rank = ULTRABLUR_RANK[source];
  if (rank < ultraBlurRank) return;
  ultraBlurRank = rank;
  usePlaybackStore.setState({ ultraBlurColors: colors });
}

export const usePlaybackStore = create<PlaybackState>((set, get) => ({
  status: "stopped",
  currentTrack: null,
  queueIndex: 0,
  position: 0,
  duration: 0,
  volume: 100,
  isBuffering: false,

  lyrics: null,
  lyricsLoading: false,
  lyricsStatus: null,
  showLyrics: false,

  waveformLevels: null,

  ultraBlurColors: null,
  vibrantPalette: null,

  queue: [],
  showQueue: false,

  nowPlayingAlbum: null,

  currentGenres: [],

  isFocusMode: false,
  visualizerMode: "bars",
  spectrumState: null,

  onPlaybackState: (status, track, queueIndex) => {
    const prev = get().currentTrack;
    const trackChanged = track?.ratingKey !== prev?.ratingKey;

    // Invalidate in-flight spectrum + lyrics refreshes so stale data from the
    // previous track cannot land on the new one. UltraBlur colours reset per
    // ALBUM, not per track: on a same-album track change the art (and the
    // views' lastAccentThumb guard) is unchanged, so extraction never
    // re-runs — reopening the gate would just let the coarser instant-paint
    // colours displace the landed art-derived ones mid-album.
    if (trackChanged) {
      spectrumGen += 1;
      lyricsGen += 1;
      if (track?.albumKey !== prev?.albumKey) resetUltraBlurGate();
    }

    set({
      status: status as PlaybackState["status"],
      currentTrack: track,
      queueIndex,
      // Seed duration from Plex metadata so the waveform and seek bar are
      // functional before mpv's first time-pos tick.
      ...(trackChanged ? { position: 0, duration: track?.duration ?? 0 } : {}),
    });

    if (trackChanged && track) {
      set({
        lyrics: null,
        lyricsStatus: null,
        waveformLevels: null,
        lyricsLoading: false,
        vibrantPalette: null,
        // A genuine track change starts fresh — drop any leftover reconnect
        // scanner (a failover reload never reaches here; it's suppressed).
        isBuffering: false,
      });

      // Do NOT clear `spectrumState` here. `refreshSpectrum` debounces
      // the "analysing" placeholder, and for cached tracks the fetch
      // resolves in ~20-80 ms so the placeholder never renders.
      get().refreshSpectrum(track.ratingKey);

      getWaveform(track.ratingKey)
        .then((levels) => {
          // Guard against a slow fetch landing after another track change —
          // without it the previous track's waveform paints onto the new one.
          if (get().currentTrack?.ratingKey === track.ratingKey) {
            set({ waveformLevels: levels });
          }
        })
        .catch((e) => console.warn("[waveform] fetch failed:", e));

      if (track.albumKey) {
        getAlbum(track.albumKey)
          .then((album) => set({ nowPlayingAlbum: album }))
          .catch(() => set({ nowPlayingAlbum: null }));
        getAlbumGenres(track.albumKey)
          .then((genres) => set({ currentGenres: genres }))
          .catch(() => set({ currentGenres: [] }));
        const blurGen = ultraBlurColorsGen();
        getAlbumColors(track.albumKey)
          .then((result) => {
            if (result.palette) {
              // Update accent CSS vars here too. Previously only
              // handleArtLoad (fullscreen / compact Now Playing image)
              // did this, which left the accent stale when a track change
              // happened while only the mini-player was visible.
              const [r, g, b] = accentFromPalette(result.palette);
              applyAccent(r, g, b);
              set({ vibrantPalette: result.palette });
            }
            // Instant paint until the art decodes: server colours beat the
            // palette-derived mapping, and the write gate stops this
            // resolution from clobbering an already-landed art-derived
            // extraction (cached art can decode before this IPC round-trip
            // resolves) or from landing on a later track.
            if (result.colors) {
              applyUltraBlurColors(result.colors, "server", blurGen);
            } else if (result.palette) {
              applyUltraBlurColors(blurColorsFromPalette(result.palette), "palette", blurGen);
            }
          })
          .catch(() => {});
      } else {
        set({ currentGenres: [], nowPlayingAlbum: null });
      }

      // The panel is "always pinned": if it's open, refetch lyrics for the
      // new track and keep it open.
      if (get().showLyrics) {
        get().loadLyrics(track.ratingKey);
      }

      getQueue()
        .then((queue) => set({ queue }))
        .catch(() => {});
    }

    if (!track) {
      set({
        lyrics: null,
        lyricsStatus: null,
        lyricsLoading: false,
        waveformLevels: null,
        showLyrics: false,
        currentGenres: [],
        nowPlayingAlbum: null,
        queue: [],
        isBuffering: false,
      });
    }
  },

  onPlaybackPosition: (position, duration) => {
    set({ position, duration });
  },

  setBuffering: (buffering) => {
    // Guard so an idempotent write doesn't churn subscribers each watchdog tick.
    if (get().isBuffering !== buffering) set({ isBuffering: buffering });
  },

  refreshSpectrum: (forRatingKey) => {
    const current = get().currentTrack;
    if (!current) return;
    if (forRatingKey && forRatingKey !== current.ratingKey) {
      // Event is for a different track (likely a prefetch). Its state
      // will hydrate when it starts playing.
      return;
    }

    const gen = spectrumGen;
    const ratingKey = current.ratingKey;

    // Debounced placeholder: only flip to "analysing" after 120 ms.
    // Cached `.spec` files resolve in ~50 ms, so debouncing avoids a
    // placeholder flash during bar-to-bar transitions. Cold analysis
    // (first play or slow decode) still gets visual feedback below
    // the "app is frozen" perception threshold.
    const placeholderTimer = window.setTimeout(() => {
      if (gen !== spectrumGen) return;
      set({ spectrumState: "analysing" });
    }, 120);

    getSpectrum(ratingKey)
      .then((state) => {
        clearTimeout(placeholderTimer);
        // Drop stale results if the track changed during the await. The
        // gen check beats `current.ratingKey` because replay/queue reload
        // could reuse the same key.
        if (gen !== spectrumGen) return;
        set({ spectrumState: state });
      })
      .catch((err) => {
        clearTimeout(placeholderTimer);
        if (gen !== spectrumGen) return;
        console.warn("[spectrum] getSpectrum failed:", err);
        set({ spectrumState: { unavailable: { reason: "Failed to load spectrum data" } } });
      });
  },

  refreshWaveform: (forRatingKey) => {
    const current = get().currentTrack;
    if (!current) return;
    if (forRatingKey && forRatingKey !== current.ratingKey) return;
    if (get().waveformLevels) return;

    const ratingKey = current.ratingKey;
    getWaveform(ratingKey)
      .then((levels) => {
        // Only land on the track this fetch was for — a slow response must
        // not paint the previous track's waveform onto the next one.
        if (levels && get().currentTrack?.ratingKey === ratingKey) {
          set({ waveformLevels: levels });
        }
      })
      .catch(() => {});
  },

  seek: (seconds) => {
    seekCmd(seconds).catch(() => {});
    set({ position: seconds });
  },

  seekFraction: (fraction) => {
    const dur = get().duration;
    if (dur > 0) {
      const seconds = fraction * dur;
      seekCmd(seconds).catch(() => {});
      set({ position: seconds });
    }
  },

  changeVolume: (volume) => {
    set({ volume });
    setVolumeCmd(volume).catch(() => {});
  },

  loadVolume: async () => {
    try {
      const vol = await getVolume();
      set({ volume: vol });
    } catch {}
  },

  loadLyrics: (ratingKey) => {
    const gen = lyricsGen;
    set({ lyricsLoading: true });
    fetchLyrics(ratingKey)
      .then((result) => {
        if (gen !== lyricsGen) return; // track changed mid-flight
        set({ lyrics: result.lyrics, lyricsStatus: result.status, lyricsLoading: false });
      })
      .catch(() => {
        if (gen !== lyricsGen) return;
        // A rejected IPC call is itself a connectivity failure; surface it
        // honestly rather than as "no lyrics found".
        set({ lyrics: null, lyricsStatus: "unreachable", lyricsLoading: false });
      });
  },

  toggleLyrics: () => {
    const { showLyrics, lyrics, lyricsLoading, lyricsStatus, currentTrack } = get();
    // Fetch on open only when we lack a definitive answer. A "notFound" is
    // definitive for the session (no re-pinging LRCLIB on every reopen);
    // "offline"/"unreachable" are worth retrying — the user may have reconnected.
    const needsFetch = !lyrics && lyricsStatus !== "notFound";
    if (!showLyrics && needsFetch && !lyricsLoading && currentTrack) {
      set({ showLyrics: true });
      get().loadLyrics(currentTrack.ratingKey);
    } else {
      set({ showLyrics: !showLyrics });
    }
  },

  toggleQueue: () => {
    const { showQueue } = get();
    if (!showQueue) {
      getQueue()
        .then((queue) => set({ queue, showQueue: true }))
        .catch(() => set({ showQueue: true }));
    } else {
      set({ showQueue: false });
    }
  },

  toggleFocusMode: () => set((s) => ({ isFocusMode: !s.isFocusMode })),

  cycleVisualizerMode: () =>
    set((s) => {
      const next: VisualizerMode =
        s.visualizerMode === "bars" ? "line" : s.visualizerMode === "line" ? "off" : "bars";
      return { visualizerMode: next };
    }),

  removeQueueItem: (index) => {
    removeFromQueueCmd(index).catch(() => {});
    set((s) => ({
      queue: s.queue.filter((_, i) => i !== index),
    }));
  },

  jumpToIndex: (index) => {
    jumpToQueueIndexCmd(index).catch(() => {});
  },

  clearQueue: () => {
    clearQueueCmd().catch(() => {});
    // Same reset the `!track` branch of onPlaybackState performs, applied up
    // front so the UI empties on the tap rather than on the IPC round-trip.
    spectrumGen += 1;
    lyricsGen += 1;
    resetUltraBlurGate();
    set({
      status: "stopped",
      currentTrack: null,
      queueIndex: 0,
      position: 0,
      duration: 0,
      queue: [],
      showQueue: false,
      lyrics: null,
      lyricsStatus: null,
      lyricsLoading: false,
      showLyrics: false,
      waveformLevels: null,
      spectrumState: null,
      currentGenres: [],
      nowPlayingAlbum: null,
      vibrantPalette: null,
      isBuffering: false,
    });
  },
}));
