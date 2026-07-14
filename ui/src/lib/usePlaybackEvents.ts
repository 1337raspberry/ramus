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

    // Timestamp of the last position tick — drives the buffering watchdog
    // below. There's no dedicated backend buffering event; we infer a stall
    // from position events drying up while the status is still "playing".
    let lastPositionAt = performance.now();
    const BUFFERING_STALE_MS = 1500;

    const u1 = listen<PlaybackStatePayload>("playback-state", (event) => {
      const { status, currentTrack, queueIndex } = event.payload;
      // Every state change earns a fresh grace window; a non-playing state
      // can never be "buffering".
      lastPositionAt = performance.now();
      store.onPlaybackState(status, currentTrack, queueIndex);
      if (status !== "playing") store.setBuffering(false);
    });
    const u2 = listen<PlaybackPositionPayload>("playback-position", (event) => {
      const { position, duration } = event.payload;
      // Audio is flowing — reset the staleness clock and drop the scanner
      // immediately (don't wait for the next watchdog tick).
      lastPositionAt = performance.now();
      store.setBuffering(false);
      store.onPlaybackPosition(position, duration);
    });
    // Emitted when a prefetched or current track finishes analysis.
    // Re-pull the spectrum only when the ratingKey matches the playing
    // track.
    const u3 = listen<SpectrumReadyPayload>("spectrum-ready", (event) => {
      store.refreshSpectrum(event.payload.ratingKey);
    });

    // While playing, if position events stop arriving for BUFFERING_STALE_MS
    // the audio has stalled (initial buffer, a mid-track network hiccup, or
    // the reload gap on a failover resume) — flip on the scanning indicator.
    const watchdog = window.setInterval(() => {
      const s = usePlaybackStore.getState();
      if (s.status !== "playing") return;
      if (performance.now() - lastPositionAt > BUFFERING_STALE_MS) {
        s.setBuffering(true);
      }
    }, 250);

    store.loadVolume();

    return () => {
      window.clearInterval(watchdog);
      u1.then((fn) => fn());
      u2.then((fn) => fn());
      u3.then((fn) => fn());
    };
  }, []);
}
