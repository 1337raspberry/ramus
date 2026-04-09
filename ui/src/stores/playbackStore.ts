import { create } from "zustand";
import type { Album, Track, LyricsResult, UltraBlurColors } from "../lib/types";
import { blurColorsFromPalette, type VibrantPalette } from "../lib/vibrantColor";
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

  // --- Event Handlers ---
  onPlaybackState: (status: string, track: Track | null, queueIndex: number) => void;
  onPlaybackPosition: (position: number, duration: number) => void;
  onBuffering: (isBuffering: boolean, bufferedFraction: number) => void;

  // --- Actions ---
  seek: (seconds: number) => void;
  seekFraction: (fraction: number) => void;
  changeVolume: (volume: number) => void;
  loadVolume: () => void;
  toggleLyrics: () => void;
  toggleLyricsPinned: () => void;
  toggleQueue: () => void;
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

  onPlaybackState: (status, track, queueIndex) => {
    const prev = get().currentTrack;
    const trackChanged = track?.ratingKey !== prev?.ratingKey;

    set({
      status: status as PlaybackState["status"],
      currentTrack: track,
      queueIndex,
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
