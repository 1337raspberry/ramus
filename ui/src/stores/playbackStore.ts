import { create } from "zustand";
import type { Album, AudioLevelPayload, LyricsResult, Track, UltraBlurColors } from "../lib/types";
import { blurColorsFromPalette, type VibrantPalette } from "../lib/vibrantColor";
import { useVisualizerDebugStore } from "./visualizerDebugStore";
import {
  getVolume,
  setVolume as setVolumeCmd,
  seek as seekCmd,
  fetchLyrics,
  getWaveform,
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
  // Whether the focus-mode visualiser layer is rendered. Session-only —
  // resets to true on reload. Controlled by the IconWave toggle next to
  // the equalizer button in FocusNowPlayingView's track row.
  showVisualizer: boolean;

  // --- Realtime audio meter (fed from mpv `af-metadata/astats`) ---
  // FocusVisualizer reads this via getState() inside a RAF loop to avoid
  // re-renders on every 30fps tick. Don't subscribe via a React selector.
  audioLevels: AudioLevelPayload | null;

  // --- Event Handlers ---
  onPlaybackState: (status: string, track: Track | null, queueIndex: number) => void;
  onPlaybackPosition: (position: number, duration: number) => void;
  onBuffering: (isBuffering: boolean, bufferedFraction: number) => void;
  onAudioLevel: (payload: AudioLevelPayload) => void;

  // --- Actions ---
  seek: (seconds: number) => void;
  seekFraction: (fraction: number) => void;
  changeVolume: (volume: number) => void;
  loadVolume: () => void;
  toggleLyrics: () => void;
  toggleLyricsPinned: () => void;
  toggleQueue: () => void;
  toggleFocusMode: () => void;
  toggleVisualizer: () => void;
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

// Monotonic generation counter for the audio-level delayed dispatch path.
// Incremented on every track change; pending setTimeouts compare against the
// current value and drop their payload if it has advanced, preventing the
// previous track's buffered meter data from bleeding into the new track
// during the ~600 ms delay window (see onAudioLevel below).
let audioLevelGen = 0;

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
  showVisualizer: true,
  audioLevels: null,

  onPlaybackState: (status, track, queueIndex) => {
    const prev = get().currentTrack;
    const trackChanged = track?.ratingKey !== prev?.ratingKey;

    // Invalidate any in-flight delayed audio-level dispatches so stale
    // metering data from the previous track can't land on the new one.
    if (trackChanged) {
      audioLevelGen += 1;
    }

    set({
      status: status as PlaybackState["status"],
      currentTrack: track,
      queueIndex,
      ...(trackChanged ? { position: 0, duration: 0 } : {}),
    });

    if (trackChanged && track) {
      set({ lyrics: null, waveformLevels: null, lyricsLoading: false, vibrantPalette: null });

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

  onAudioLevel: (payload) => {
    // Hot-path mutation: set() is still fine because no React component
    // subscribes to audioLevels via a selector — FocusVisualizer reads via
    // getState() inside requestAnimationFrame.
    //
    // Visual delay: astats sees PCM samples upstream of the OS audio sink,
    // so the visualiser naturally leads the speakers by ~mpv audio-buffer
    // (~500 ms). Buffering the event by a matching delay realigns the
    // visual reaction with what the listener actually hears.
    //
    // DEBUG (focus visualiser panel): currently reads the delay from the
    // live debug store so the slider can tune it. When the debug panel is
    // removed, replace the `useVisualizerDebugStore.getState().visualDelayMs`
    // read with `VISUALIZER_DEFAULTS.visualDelayMs` imported from
    // `./visualizerDebugStore`.
    const delay = useVisualizerDebugStore.getState().visualDelayMs;
    if (delay > 0) {
      // Capture the current generation in the closure. If the track
      // changes before this timeout fires, the generation will have
      // advanced and we drop the stale payload.
      const gen = audioLevelGen;
      setTimeout(() => {
        if (gen !== audioLevelGen) return;
        set({ audioLevels: payload });
      }, delay);
    } else {
      set({ audioLevels: payload });
    }
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

  toggleVisualizer: () => set((s) => ({ showVisualizer: !s.showVisualizer })),

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
