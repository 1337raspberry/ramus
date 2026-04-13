import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import type {
  AccentColorPayload,
  PlaybackStatePayload,
  PlaybackPositionPayload,
  PlaybackBufferingPayload,
  SpectrumReadyPayload,
} from "./types";
import { usePlaybackStore } from "../stores/playbackStore";

/**
 * Subscribe to Tauri playback events (accent-color, playback-state,
 * playback-position, playback-buffering, spectrum-ready) and load
 * the saved volume on mount. Side-effect only — returns nothing.
 */
export function usePlaybackEvents(): void {
  // Listen for accent color events
  useEffect(() => {
    const unlisten = listen<AccentColorPayload>("accent-color", (event) => {
      const { r, g, b } = event.payload;
      document.documentElement.style.setProperty("--accent-r", String(r));
      document.documentElement.style.setProperty("--accent-g", String(g));
      document.documentElement.style.setProperty("--accent-b", String(b));
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Listen for playback events
  useEffect(() => {
    const store = usePlaybackStore.getState();

    const u1 = listen<PlaybackStatePayload>("playback-state", (event) => {
      const { status, currentTrack, queueIndex } = event.payload;
      store.onPlaybackState(status, currentTrack, queueIndex);
    });
    const u2 = listen<PlaybackPositionPayload>("playback-position", (event) => {
      const { position, duration } = event.payload;
      store.onPlaybackPosition(position, duration);
    });
    const u3 = listen<PlaybackBufferingPayload>("playback-buffering", (event) => {
      const { isBuffering, bufferedFraction } = event.payload;
      store.onBuffering(isBuffering, bufferedFraction);
    });
    // Focus-mode spectrum: Rust emits this when a prefetched track or
    // the current track finishes analysis. Re-pull the spectrum for the
    // currently playing track if it matches; otherwise ignore.
    const u4 = listen<SpectrumReadyPayload>("spectrum-ready", (event) => {
      store.refreshSpectrum(event.payload.ratingKey);
    });

    store.loadVolume();

    return () => {
      u1.then((fn) => fn());
      u2.then((fn) => fn());
      u3.then((fn) => fn());
      u4.then((fn) => fn());
    };
  }, []);
}
