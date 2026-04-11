import { create } from "zustand";
import type { Album, LyricsResult, SpectrumState, Track, UltraBlurColors } from "../lib/types";
import { blurColorsFromPalette, type VibrantPalette } from "../lib/vibrantColor";

/**
 * Which rendering mode the focus-mode visualiser is in.
 *
 * - `"off"`  — viz is unmounted entirely (RAF loop stops)
 * - `"bars"` — 256-bar mirrored spectrum, bass centred, treble at edges
 * - `"line"` — smoothed averaged curve filled from the top edge down
 *
 * Cycled by clicking the wave-icon button in the focus-mode track row;
 * see `cycleVisualizerMode`.
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
} from "../lib/commands";

interface PlaybackState {
  // --- Playback ---
  status: "stopped" | "playing" | "paused";
  currentTrack: Track | null;
  queueIndex: number;
  position: number;
  duration: number;
  isBuffering: boolean;
  bufferedFraction: number;
  volume: number;

  // --- Lyrics ---
  lyrics: LyricsResult | null;
  lyricsLoading: boolean;
  showLyrics: boolean;
  lyricsPinned: boolean;

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
  // Which visualiser rendering mode is active in focus mode. Session-
  // only — resets to `"bars"` on reload. Cycled `bars → line → off` by
  // clicking the IconWave button next to the equalizer in
  // FocusNowPlayingView's track row (see `cycleVisualizerMode`).
  visualizerMode: VisualizerMode;

  // --- Focus-mode FFT spectrogram ---
  //
  // Precomputed per-track bands from symphonia + realfft in Rust.
  // Hydrated on track change and on every `spectrum-ready` event.
  // FocusVisualizer reads this via getState() inside a RAF loop to avoid
  // re-renders on every 60fps tick. Don't subscribe via a React selector.
  //
  // `null` = never fetched for the current track (before the first
  // getSpectrum call). `"analysing"` = backend knows the track but
  // hasn't finished analysis yet; the viz shows a placeholder. When it
  // flips to `{ ready }` the viz starts drawing bars at the current
  // `position` lookup.
  spectrumState: SpectrumState | null;

  // --- Event Handlers ---
  onPlaybackState: (status: string, track: Track | null, queueIndex: number) => void;
  onPlaybackPosition: (position: number, duration: number) => void;
  onBuffering: (isBuffering: boolean, bufferedFraction: number) => void;
  /// Called on `spectrum-ready` events from Rust AND once at track change
  /// to hydrate from the cache. Safe to call unconditionally; it only
  /// invokes `getSpectrum` when there's a current track.
  refreshSpectrum: (forRatingKey?: string) => void;

  // --- Actions ---
  seek: (seconds: number) => void;
  seekFraction: (fraction: number) => void;
  changeVolume: (volume: number) => void;
  loadVolume: () => void;
  toggleLyrics: () => void;
  toggleLyricsPinned: () => void;
  toggleQueue: () => void;
  toggleFocusMode: () => void;
  cycleVisualizerMode: () => void;
  removeQueueItem: (index: number) => void;
  jumpToIndex: (index: number) => void;
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

// Monotonic generation counter for the async spectrum-refresh path. Any
// in-flight `getSpectrum` invoke checks this against the value captured
// when it started — if the track has changed in the meantime, the
// result is dropped so stale data from the previous track can't bleed
// into the new track's UI state.
let spectrumGen = 0;

export const usePlaybackStore = create<PlaybackState>((set, get) => ({
  status: "stopped",
  currentTrack: null,
  queueIndex: 0,
  position: 0,
  duration: 0,
  isBuffering: false,
  bufferedFraction: 0,
  volume: 100,

  lyrics: null,
  lyricsLoading: false,
  showLyrics: false,
  lyricsPinned: false,

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

    // Invalidate any in-flight spectrum refreshes so stale data from the
    // previous track can't land on the new one.
    if (trackChanged) {
      spectrumGen += 1;
    }

    set({
      status: status as PlaybackState["status"],
      currentTrack: track,
      queueIndex,
      ...(trackChanged ? { position: 0, duration: 0 } : {}),
    });

    if (trackChanged && track) {
      set({
        lyrics: null,
        waveformLevels: null,
        lyricsLoading: false,
        vibrantPalette: null,
      });

      // `refreshSpectrum` below will debounce the "analysing"
      // placeholder itself — see the comment there. We intentionally
      // do NOT clear `spectrumState` here, because for cached tracks
      // the fetch resolves in ~20-80 ms and debouncing avoids the
      // placeholder ever rendering for those.
      get().refreshSpectrum(track.ratingKey);

      getWaveform(track.ratingKey)
        .then((levels) => {
          if (levels) {
            console.log(`[waveform] got ${levels.length} levels for ${track.ratingKey}`);
          } else {
            console.log(`[waveform] no levels returned for ${track.ratingKey}`);
          }
          set({ waveformLevels: levels });
        })
        .catch((e) => console.warn("[waveform] fetch failed:", e));

      if (track.albumKey) {
        getAlbum(track.albumKey)
          .then((album) => set({ nowPlayingAlbum: album }))
          .catch(() => set({ nowPlayingAlbum: null }));
        getAlbumGenres(track.albumKey)
          .then((genres) => set({ currentGenres: genres }))
          .catch(() => set({ currentGenres: [] }));
        getAlbumColors(track.albumKey)
          .then((result) => {
            if (result.palette) {
              set({
                vibrantPalette: result.palette,
                ultraBlurColors: blurColorsFromPalette(result.palette),
              });
            } else if (result.colors) {
              set({ ultraBlurColors: result.colors });
            }
          })
          .catch(() => {});
      } else {
        set({ currentGenres: [], nowPlayingAlbum: null });
      }

      // Auto-fetch lyrics if pinned
      if (get().lyricsPinned) {
        set({ lyricsLoading: true });
        fetchLyrics(track.ratingKey)
          .then((result) => set({ lyrics: result, lyricsLoading: false }))
          .catch(() => set({ lyricsLoading: false }));
      }

      getQueue()
        .then((queue) => set({ queue }))
        .catch(() => {});
    }

    if (!track) {
      set({
        lyrics: null,
        waveformLevels: null,
        showLyrics: false,
        currentGenres: [],
        nowPlayingAlbum: null,
        queue: [],
      });
    }
  },

  onPlaybackPosition: (position, duration) => {
    set({ position, duration });
  },

  onBuffering: (isBuffering, bufferedFraction) => {
    set({ isBuffering, bufferedFraction });
  },

  refreshSpectrum: (forRatingKey) => {
    // Resolve the target rating key: either the caller-provided one
    // (from a spectrum-ready event, where we want to match it against
    // the current track) or the current track's key.
    const current = get().currentTrack;
    if (!current) return;
    if (forRatingKey && forRatingKey !== current.ratingKey) {
      // The event was for a different track (probably the next
      // prefetched one). Ignore — its state will hydrate when it
      // actually starts playing.
      return;
    }

    const gen = spectrumGen;
    const ratingKey = current.ratingKey;
    const trackDurationS = current.duration;

    // Debounced placeholder: only flip to "analysing" if the fetch
    // takes more than 120 ms. Rationale: for cached `.spec` files the
    // Tauri IPC resolves in ~50 ms and we want seamless bar-to-bar
    // transitions without a placeholder flash. For cold analysis
    // (first play of an uncached track, or a slow decode) we do want
    // visual feedback, and 120 ms is under the threshold where users
    // perceive "the app is frozen".
    let placeholderFired = false;
    const placeholderTimer = window.setTimeout(() => {
      if (gen !== spectrumGen) return;
      placeholderFired = true;
      set({ spectrumState: "analysing" });
    }, 120);

    const t0 = performance.now();
    getSpectrum(ratingKey)
      .then((state) => {
        const ipcMs = performance.now() - t0;
        clearTimeout(placeholderTimer);
        // Drop stale results if the track has changed while we were
        // waiting. The gen check beats comparing current.ratingKey
        // because a new track could theoretically have the same key
        // (replay / queue reload).
        if (gen !== spectrumGen) return;

        // Timing sanity log: IPC latency is the key number here. If
        // it's routinely over 500 ms for cached tracks, the JSON-array
        // encoding of `Vec<u8>` is the bottleneck and we should switch
        // to binary IPC via `tauri::ipc::Response`. Also
        // cross-reference the spectrogram's own sense of duration
        // against Plex's — large deltas mean the analyser is off.
        if (typeof state === "object" && "ready" in state) {
          const frames = state.ready;
          const frameCount = Math.floor(frames.frames.length / frames.bandCount);
          const analyserDurationS = (frameCount * frames.hopMs) / 1000;
          console.log(
            `[spectrum] ${ratingKey} ready: ` +
              `ipc=${ipcMs.toFixed(0)}ms ` +
              `placeholderShown=${placeholderFired} ` +
              `hopMs=${frames.hopMs.toFixed(3)} ` +
              `bands=${frames.bandCount} ` +
              `sr=${frames.sampleRate} ` +
              `frames=${frameCount} ` +
              `analyserDuration=${analyserDurationS.toFixed(2)}s ` +
              `plexDuration=${trackDurationS.toFixed(2)}s ` +
              `delta=${(analyserDurationS - trackDurationS).toFixed(2)}s`,
          );
        } else {
          console.log(
            `[spectrum] ${ratingKey} ${typeof state === "string" ? state : "unavailable"}: ` +
              `ipc=${ipcMs.toFixed(0)}ms placeholderShown=${placeholderFired}`,
          );
        }

        // Measure the state-commit cost too — this is where
        // useMemo's `Uint8Array.from` runs in FocusVisualizer, so a
        // large value here points at the JSON-array conversion being
        // the bottleneck rather than the IPC itself.
        const tBeforeSet = performance.now();
        set({ spectrumState: state });
        const setMs = performance.now() - tBeforeSet;
        if (setMs > 30) {
          console.log(`[spectrum] ${ratingKey} set() took ${setMs.toFixed(0)}ms`);
        }
      })
      .catch((err) => {
        clearTimeout(placeholderTimer);
        if (gen !== spectrumGen) return;
        console.warn("[spectrum] getSpectrum failed:", err);
        set({ spectrumState: "analysing" });
      });
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

  toggleLyrics: () => {
    const { showLyrics, lyrics, lyricsLoading, currentTrack } = get();
    if (!showLyrics && !lyrics && !lyricsLoading && currentTrack) {
      set({ lyricsLoading: true, showLyrics: true });
      fetchLyrics(currentTrack.ratingKey)
        .then((result) => set({ lyrics: result, lyricsLoading: false }))
        .catch(() => set({ lyricsLoading: false }));
    } else {
      set({ showLyrics: !showLyrics });
    }
  },

  toggleLyricsPinned: () => set((s) => ({ lyricsPinned: !s.lyricsPinned })),

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
}));
