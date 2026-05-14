import { create } from "zustand";
import type { Album, LyricsResult, SpectrumState, Track, UltraBlurColors } from "../lib/types";
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
} from "../lib/commands";

interface PlaybackState {
  // --- Playback ---
  status: "stopped" | "playing" | "paused";
  currentTrack: Track | null;
  queueIndex: number;
  position: number;
  duration: number;
  volume: number;

  // --- Lyrics ---
  lyrics: LyricsResult | null;
  lyricsLoading: boolean;
  showLyrics: boolean;
  lyricsPinned: boolean;

  // --- Waveform ---
  waveformLevels: number[] | null;
  // Android-only — set true while the Tauri plugin is pre-downloading
  // a chunked Plex transcode (`/transcode/universal/start` doesn't
  // support Range, so ExoPlayer can't seek the live stream). Drives
  // the scanning-bar overlay on `WaveformSeekBar`. Other platforms
  // never see this flip; the wiring is no-op there.
  isBuffering: boolean;

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
  /// Called on `spectrum-ready` events from Rust and on track change to
  /// hydrate from the cache. Safe to call unconditionally; it only invokes
  /// `getSpectrum` when there is a current track.
  refreshSpectrum: (forRatingKey?: string) => void;

  // --- Actions ---
  setBuffering: (buffering: boolean) => void;
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

// Monotonic generation counter for async spectrum refreshes. In-flight
// `getSpectrum` invokes compare against the captured value and drop their
// result if the track has changed.
let spectrumGen = 0;

export const usePlaybackStore = create<PlaybackState>((set, get) => ({
  status: "stopped",
  currentTrack: null,
  queueIndex: 0,
  position: 0,
  duration: 0,
  volume: 100,

  lyrics: null,
  lyricsLoading: false,
  showLyrics: false,
  lyricsPinned: false,

  waveformLevels: null,
  isBuffering: false,

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

    // Invalidate in-flight spectrum refreshes so stale data from the
    // previous track cannot land on the new one.
    if (trackChanged) {
      spectrumGen += 1;
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
        waveformLevels: null,
        lyricsLoading: false,
        vibrantPalette: null,
      });

      // Do NOT clear `spectrumState` here. `refreshSpectrum` debounces
      // the "analysing" placeholder, and for cached tracks the fetch
      // resolves in ~20-80 ms so the placeholder never renders.
      get().refreshSpectrum(track.ratingKey);

      getWaveform(track.ratingKey)
        .then((levels) => set({ waveformLevels: levels }))
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
              // Update accent CSS vars here too. Previously only
              // handleArtLoad (fullscreen / compact Now Playing image)
              // did this, which left the accent stale when a track change
              // happened while only the mini-player was visible.
              const [r, g, b] = accentFromPalette(result.palette);
              applyAccent(r, g, b);
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

      if (get().lyricsPinned) {
        set({ lyricsLoading: true });
        fetchLyrics(track.ratingKey)
          .then((result) => set({ lyrics: result, lyricsLoading: false }))
          .catch(() => set({ lyricsLoading: false }));
      } else if (get().showLyrics) {
        set({ showLyrics: false });
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

  setBuffering: (buffering) => set({ isBuffering: buffering }),

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
