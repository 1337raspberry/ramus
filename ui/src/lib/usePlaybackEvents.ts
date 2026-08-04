import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import type {
  AccentColorPayload,
  PlaybackStatePayload,
  PlaybackPositionPayload,
  PlaybackBufferingPayload,
  SpectrumReadyPayload,
  MetadataWarmedPayload,
} from "./types";
import { usePlaybackStore } from "../stores/playbackStore";
import { useConnectionStore } from "../stores/connectionStore";
import { applyAccent } from "./accent";
import { bumpArtRetry } from "./useArtUrl";

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
    // A background warm landed a metadata artefact — retry any surface still
    // showing a placeholder. Art bumps the global retry epoch (only
    // unresolved useArtUrl slots refetch, and they hit the freshly-warmed
    // disk cache); a waveform re-pulls only when it's for the current track
    // and the seek bar is still empty.
    const u5 = listen<MetadataWarmedPayload>("metadata-warmed", (event) => {
      if (event.payload.kind === "waveform") {
        store.refreshWaveform(event.payload.ratingKey ?? undefined);
      } else {
        bumpArtRetry();
      }
    });

    // Connection recovery: fetches that failed while the link was down (or
    // black-holed mid-drive) are worth one retry now. The backend's
    // reconnect handler also kicks a fresh prefetch cycle, whose warm tier
    // re-fetches anything still missing and re-fires `metadata-warmed`.
    let wasOnline = (() => {
      const c = useConnectionStore.getState();
      return c.online && !c.effectiveOffline;
    })();
    const unsubConnection = useConnectionStore.subscribe((c) => {
      const nowOnline = c.online && !c.effectiveOffline;
      if (!wasOnline && nowOnline) {
        bumpArtRetry();
        store.refreshWaveform();
      }
      wasOnline = nowOnline;
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
      unsubConnection();
      u1.then((fn) => fn());
      u2.then((fn) => fn());
      u3.then((fn) => fn());
      u4.then((fn) => fn());
      u5.then((fn) => fn());
    };
  }, []);
}
