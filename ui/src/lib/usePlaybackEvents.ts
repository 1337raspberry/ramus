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
    // below. The watchdog is the fallback heuristic for ordinary stalls
    // (position events drying up while status is "playing"); the dedicated
    // backend `playback-buffering` event handled further down covers the
    // failover-reload gap this heuristic can't see.
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
    // Backend-driven buffering signal for the reconnect/reload gap (a
    // connection failover or file-ended resume). The frontend can't infer
    // these from position starvation — the status stays "playing" and no
    // position events flow through the gap — so the backend tells us directly.
    // The scanner is cleared again by the next real position tick (above) or a
    // non-playing state (the watchdog only ever sets it, never unsets).
    const u4 = listen<PlaybackBufferingPayload>("playback-buffering", (event) => {
      // A non-playing player can never be "buffering" (same invariant the
      // watchdog and state handler enforce). A failover reload of a PAUSED
      // remote track still emits true, and with no position ticks coming
      // nothing would ever clear the scanner.
      if (event.payload.buffering && usePlaybackStore.getState().status !== "playing") return;
      store.setBuffering(event.payload.buffering);
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
      u4.then((fn) => fn());
    };
  }, []);
}
