import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import type {
  AccentColorPayload,
  PlaybackStatePayload,
  PlaybackPositionPayload,
  SpectrumReadyPayload,
} from "./types";
import { usePlaybackStore } from "../stores/playbackStore";
import { applyAccent } from "./accent";

/**
 * Subscribe to Tauri playback events (accent-color, playback-state,
 * playback-position, spectrum-ready) and load the saved volume on mount.
 */
export function usePlaybackEvents(): void {
  useEffect(() => {
    const unlisten = listen<AccentColorPayload>("accent-color", (event) => {
      const { r, g, b } = event.payload;
      applyAccent(r, g, b);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

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
    // Emitted when a prefetched or current track finishes analysis.
    // Re-pull the spectrum only when the ratingKey matches the playing
    // track.
    const u3 = listen<SpectrumReadyPayload>("spectrum-ready", (event) => {
      store.refreshSpectrum(event.payload.ratingKey);
    });
    store.loadVolume();

    return () => {
      u1.then((fn) => fn());
      u2.then((fn) => fn());
      u3.then((fn) => fn());
    };
  }, []);
}
