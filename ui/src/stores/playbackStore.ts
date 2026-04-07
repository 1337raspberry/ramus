import { create } from "zustand";
import type { Track, LyricsResult } from "../lib/types";
import {
  getVolume,
  setVolume as setVolumeCmd,
  seek as seekCmd,
  fetchLyrics,
  getWaveform,
  getQueue,
  getAlbumGenres,
  removeFromQueue as removeFromQueueCmd,
  jumpToQueueIndex as jumpToQueueIndexCmd,
} from "../lib/commands";

interface PlaybackState {
  // Playback
  status: "stopped" | "playing" | "paused";
  currentTrack: Track | null;
  queueIndex: number;
  position: number;
  duration: number;
  isBuffering: boolean;
  bufferedFraction: number;
  volume: number;

  // Lyrics
  lyrics: LyricsResult | null;
  lyricsLoading: boolean;
  showLyrics: boolean;
  lyricsPinned: boolean;

  // Waveform
  waveformLevels: number[] | null;

  // Queue
  queue: Track[];
  showQueue: boolean;

  // Album genres for now-playing footer
  currentGenres: string[];

  // Event handlers (called from App.tsx)
  onPlaybackState: (status: string, track: Track | null, queueIndex: number) => void;
  onPlaybackPosition: (position: number, duration: number) => void;
  onBuffering: (isBuffering: boolean, bufferedFraction: number) => void;

  // Actions
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

  queue: [],
  showQueue: false,

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
      // Reset lyrics/waveform for new track
      set({ lyrics: null, waveformLevels: null, lyricsLoading: false });

      // Fetch waveform
      getWaveform(track.ratingKey)
        .then((levels) => set({ waveformLevels: levels }))
        .catch(() => {});

      // Fetch genres for the album
      if (track.albumKey) {
        getAlbumGenres(track.albumKey)
          .then((genres) => set({ currentGenres: genres }))
          .catch(() => set({ currentGenres: [] }));
      } else {
        set({ currentGenres: [] });
      }

      // Auto-fetch lyrics if pinned
      if (get().lyricsPinned) {
        set({ lyricsLoading: true });
        fetchLyrics(track.ratingKey)
          .then((result) => set({ lyrics: result, lyricsLoading: false }))
          .catch(() => set({ lyricsLoading: false }));
      }

      // Refresh queue
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
    } catch {
      // ignore
    }
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
