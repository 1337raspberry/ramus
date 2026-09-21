import { useEffect, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import { setSpectrumTap } from "../lib/commands";
import { pickSpectrumFrame, spectrumLastPushAt } from "../lib/spectrumRing";
import { VISUALIZER_PARAMS } from "../lib/visualizerParams";
import { accentFromPalette } from "../lib/vibrantColor";
import { DEFAULT_ACCENT } from "../lib/accent";

/**
 * Focus-mode live spectrum visualiser.
 *
 * Data source: the `spectrum-frames` event, fed by a libavfilter tap in
 * mpv's `--af` chain (`ramus-core/src/playback/spectrum_tap.rs`). Mounting
 * installs the tap via `setSpectrumTap(true)`; unmounting removes it, so
 * its CPU cost is only paid while the visualiser is on screen. Frames
 * land in `lib/spectrumRing.ts` keyed by track position, up to ~0.5 s
 * ahead of the reported playback position.
 *
 * Sync: each paint estimates the playhead as the last position tick plus
 * the wall-clock elapsed since it (while playing) and draws the ring
 * frame nearest that estimate. Because the frames carry mpv's own
 * timestamps, seeks, pauses, stalls and track changes need no special
 * handling here: no frame near the estimated position means no bars.
 *
 * Rendering: one bar per band per channel. A frame carries the left
 * channel's N bands followed by the right channel's (N is 64 by default),
 * drawn as a stereo mirror: the left channel on the left half with its
 * bass at the centre, the right channel on the right half likewise, so
 * treble sits at both edges. No interpolation and no synthetic jitter:
 * every bar is a measured band, and the two halves differ exactly as
 * much as the mix does.
 *
 *     bar 0      → left  band N-1 (highest, far left)
 *     bar N-1    → left  band 0   (lowest, just left of centre)
 *     bar N      → right band 0   (lowest, just right of centre)
 *     bar 2N-1   → right band N-1 (highest, far right)
 *
 * The bar count comes from the frames themselves; buffers are sized on
 * the first frame and resized if it ever changes.
 */

/** Bands per channel assumed until the first frame arrives. */
const DEFAULT_BAND_COUNT = 64;
/** Channels per frame (left, right). */
const CHANNELS = 2;

/** Bar geometry, opacity, easing and level curve: `lib/visualizerParams.ts`. */
const MIN_VISIBLE_HEIGHT_PX = 0.5;
const GRADIENT_TOP_OPACITY = 0.95;
const BORDER_WIDTH_PX = 0;
const BORDER_OPACITY = 0;

/**
 * A frame this far behind the estimated playhead is still drawn. Covers
 * position-tick jitter and a dropped burst without falling back to
 * silence between bursts.
 */
const FRAME_LAG_TOLERANCE_S = 0.25;
/**
 * A frame this far ahead of the estimated playhead is still drawn. About
 * three frames: absorbs the IPC latency between a position tick being
 * taken and its arrival, which makes the estimate run slightly behind.
 */
const FRAME_LEAD_TOLERANCE_S = 0.05;
/**
 * Audio has been flowing for this long with no frames at all → the tap
 * is not producing (the libmpv build lacks the analysis filters, or the
 * graph failed to configure). Show a hint instead of a silent blank.
 */
const NO_FRAMES_HINT_MS = 3000;

/**
 * Copy one stereo frame's band heights onto the bars in `out`, one bar
 * per value: the left channel's bands reversed onto the left half so its
 * bass sits at the centre, the right channel's bands in order onto the
 * right half. `out` must be exactly as long as the frame.
 */
function readBandsInto(bands: Uint8Array, out: Uint8Array): void {
  const total = bands.length;
  const n = total / CHANNELS;
  if (out.length !== total || !Number.isInteger(n)) {
    out.fill(0);
    return;
  }
  for (let i = 0; i < n; i++) {
    out[i] = bands[n - 1 - i];
    out[n + i] = bands[n + i];
  }
}

/**
 * Install and remove requests are chained so they reach the backend in
 * mount order, one at a time. A remount issues remove-then-install with no
 * gap (React runs an effect's cleanup and re-run back to back in
 * StrictMode, and a fast toggle does the same), and each command runs on
 * its own backend task, so unchained requests could complete in the
 * opposite order and leave the tap off while the visualiser is mounted.
 */
let tapRequests: Promise<void> = Promise.resolve();
function requestTap(enabled: boolean): void {
  tapRequests = tapRequests
    .then(() => setSpectrumTap(enabled))
    .catch((e) => console.warn(`[spectrum] tap ${enabled ? "install" : "remove"} failed:`, e));
}

export default function FocusVisualizer() {
  const disabled = useSettingsStore((s) => s.disableSpectrum);

  // The tap is installed for exactly as long as this component is mounted
  // with the visualiser enabled. Disabling it in settings while mounted
  // runs the cleanup, which removes the tap; the backend applies the same
  // veto on its side, so a stale install request can't slip through.
  useEffect(() => {
    if (disabled) return;
    requestTap(true);
    return () => requestTap(false);
  }, [disabled]);

  if (disabled) return null;

  return <CanvasLayer />;
}

// --- Canvas layer ---

function CanvasLayer() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number>(0);

  // "Playing but no frames" hint. Only the transitions reach React; the
  // RAF loop compares against the ref each paint.
  const [starved, setStarved] = useState(false);
  const starvedRef = useRef(false);

  // Cached accent RGB as pre-stringified channel values for rgba()
  // template literals. Refreshed by the effect below on vibrantPalette
  // change (at most once per track). Avoids calling
  // `getComputedStyle(document.documentElement)` on every frame, which
  // would trigger a style-recalc 60 times a second.
  const accentRef = useRef<{ r: string; g: string; b: string }>({
    r: "120",
    g: "90",
    b: "220",
  });
  const vibrantPalette = usePlaybackStore((s) => s.vibrantPalette);
  const backgroundStyle = useSettingsStore((s) => s.backgroundStyle);
  useEffect(() => {
    // Only `defaultColours` locks the visualizer to the brand accent;
    // `oledVoid` blacks out the backdrop but keeps the art-derived accent.
    if (backgroundStyle === "defaultColours") {
      const [r, g, b] = DEFAULT_ACCENT;
      accentRef.current = { r: String(r), g: String(g), b: String(b) };
      return;
    }
    if (!vibrantPalette) {
      // Fresh session or a track with no art. Match the CSS `:root`
      // defaults.
      accentRef.current = { r: "120", g: "90", b: "220" };
      return;
    }
    const [r, g, b] = accentFromPalette(vibrantPalette);
    accentRef.current = { r: String(r), g: String(g), b: String(b) };
  }, [vibrantPalette, backgroundStyle]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // Track DPR and dimensions so the backing store resizes cleanly
    // when moving between displays (e.g. 1x to 2x Retina).
    let lastW = 0;
    let lastH = 0;
    let lastDpr = 0;

    const resize = () => {
      const rect = container.getBoundingClientRect();
      const w = Math.max(1, Math.floor(rect.width));
      const h = Math.max(1, Math.floor(rect.height));
      const dpr = window.devicePixelRatio || 1;
      if (w !== lastW || h !== lastH || dpr !== lastDpr) {
        canvas.width = w * dpr;
        canvas.height = h * dpr;
        canvas.style.width = `${w}px`;
        canvas.style.height = `${h}px`;
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        lastW = w;
        lastH = h;
        lastDpr = dpr;
      }
      return { w, h };
    };

    const resizeObs = new ResizeObserver(() => resize());
    resizeObs.observe(container);

    // Per-bar buffers, one entry per band per channel. `current` is the
    // eased height each bar is drawn at (so bars decay smoothly between
    // frames rather than snapping); `scratch` receives the target frame.
    // Both are reallocated, and the eased heights start from silence, if
    // the frame width ever changes.
    let current = new Float32Array(CHANNELS * DEFAULT_BAND_COUNT);
    let scratch = new Uint8Array(CHANNELS * DEFAULT_BAND_COUNT);

    // Wall-clock of the mount; the "no frames" hint waits this long after
    // mounting as well as after the last frame, so the tap has time to
    // install before it is judged.
    const mountedAt = performance.now();

    // Timestamp of the previous RAF callback; feeds `rawDelta` for
    // time-normalised easing.
    let lastTs = 0;

    // The attack and decay factors are tuned against a 60Hz reference.
    // Rescale per actual frame dt via
    //   alpha = 1 - (1 - ease) ^ (dt / referenceDt)
    // so wall-clock behaviour matches across 60/120/360 Hz displays.
    // Without this the lerp converges 6x faster on a 360Hz display,
    // collapsing the rise/fall into a snap+plateau pattern.
    const EASE_REFERENCE_DT_MS = 1000 / 60;
    // Clamp the dt fed into alpha math so long hitches (tab
    // backgrounded, GC pause, debugger stop) don't push Math.pow into
    // extreme values. ~100ms (about 6 reference frames) already yields
    // alpha ≈ 1.
    const EASE_DT_CLAMP_MS = 100;

    const render = () => {
      const { w, h } = resize();
      ctx.clearRect(0, 0, w, h);

      // Frame delta for time-normalised easing.
      const now = performance.now();
      const rawDelta = lastTs !== 0 ? now - lastTs : 0;
      lastTs = now;

      // Hot-path reads. Position is the ground truth for the frame
      // lookup — never subscribe via a React selector.
      const playback = usePlaybackStore.getState();
      const isPlaying = playback.status === "playing";

      let haveFrame = false;
      if (isPlaying) {
        // Extrapolate from the last position tick; ticks land several
        // times a second, which is far too coarse on its own.
        const estimate = playback.position + (now - playback.positionAt) / 1000;
        const frame = pickSpectrumFrame(estimate, FRAME_LAG_TOLERANCE_S, FRAME_LEAD_TOLERANCE_S);
        if (frame) {
          if (frame.bands.length !== current.length) {
            current = new Float32Array(frame.bands.length);
            scratch = new Uint8Array(frame.bands.length);
          }
          readBandsInto(frame.bands, scratch);
          haveFrame = true;
        }
      }
      if (!haveFrame) {
        // Paused, stopped, stalled, or between bursts: decay toward zero
        // so bars don't freeze at the last value.
        scratch.fill(0);
      }

      // Audio flowing (playing, not in a buffering gap) with no frames
      // arriving at all means the tap isn't producing.
      const starvedNow =
        isPlaying &&
        !playback.isBuffering &&
        now - Math.max(spectrumLastPushAt(), mountedAt) > NO_FRAMES_HINT_MS;
      if (starvedNow !== starvedRef.current) {
        starvedRef.current = starvedNow;
        setStarved(starvedNow);
      }

      // Shape each bar's level, then spring-ease toward it. The backend
      // delivers dB position through its own compression curve; the
      // floor cut, gamma and gain here are the display-side adjustments
      // (see lib/visualizerParams.ts). Easing alphas are computed once
      // per frame, not per bar.
      const {
        floorCut,
        gamma,
        gain,
        easeAttack,
        easeDecay,
        barAlpha,
        barMaxHeight,
        barTipOpacity,
        barGap,
        barSpan,
      } = VISUALIZER_PARAMS;
      const shape = floorCut > 0 || gamma !== 1 || gain !== 1;
      const barCount = current.length;
      const easeDt = Math.min(rawDelta, EASE_DT_CLAMP_MS);
      const dtRatio = easeDt / EASE_REFERENCE_DT_MS;
      const alphaAttack = easeDt > 0 ? 1 - Math.pow(1 - easeAttack, dtRatio) : 0;
      const alphaDecay = easeDt > 0 ? 1 - Math.pow(1 - easeDecay, dtRatio) : 0;
      for (let i = 0; i < barCount; i++) {
        let target = scratch[i] / 255;
        if (shape) {
          target = target <= floorCut ? 0 : (target - floorCut) / (1 - floorCut);
          if (gamma !== 1) target = Math.pow(target, gamma);
          target = Math.min(1, target * gain);
        }
        const prev = current[i];
        const alpha = target > prev ? alphaAttack : alphaDecay;
        current[i] = prev + (target - prev) * alpha;
      }

      // Accent colour from the cached ref; no per-frame style-recalc.
      const { r, g, b } = accentRef.current;

      ctx.globalAlpha = barAlpha;

      // Gradient: opaque at the top edge where bars originate, fading
      // toward their tips.
      const maxH = h * barMaxHeight;
      const grad = ctx.createLinearGradient(0, 0, 0, maxH);
      grad.addColorStop(0, `rgba(${r}, ${g}, ${b}, ${GRADIENT_TOP_OPACITY})`);
      grad.addColorStop(1, `rgba(${r}, ${g}, ${b}, ${barTipOpacity})`);
      ctx.fillStyle = grad;

      // Stroke is applied per-bar after the fill, only when width > 0.
      const drawBorder = BORDER_WIDTH_PX > 0 && BORDER_OPACITY > 0;
      if (drawBorder) {
        ctx.lineWidth = BORDER_WIDTH_PX;
        ctx.strokeStyle = `rgba(${r}, ${g}, ${b}, ${BORDER_OPACITY})`;
      }

      // Even spacing across the bar field (the full width by default,
      // centred when narrower); recomputed per frame for resize.
      const fieldW = w * barSpan;
      const fieldX = (w - fieldW) / 2;
      const totalGap = barGap * (barCount - 1);
      const barWidth = Math.max(0.5, (fieldW - totalGap) / barCount);

      for (let i = 0; i < barCount; i++) {
        const barHeight = current[i] * maxH;
        if (barHeight < MIN_VISIBLE_HEIGHT_PX) continue;
        const x = fieldX + i * (barWidth + barGap);
        ctx.fillRect(x, 0, barWidth, barHeight);
        if (drawBorder) {
          ctx.strokeRect(x, 0, barWidth, barHeight);
        }
      }

      // Reset globalAlpha for any future shared-canvas code paths.
      ctx.globalAlpha = 1.0;

      rafRef.current = requestAnimationFrame(render);
    };

    rafRef.current = requestAnimationFrame(render);

    return () => {
      cancelAnimationFrame(rafRef.current);
      resizeObs.disconnect();
    };
  }, []);

  return (
    <div ref={containerRef} className="focus-visualizer">
      <canvas ref={canvasRef} />
      {starved && (
        <div className="focus-visualizer focus-visualizer-placeholder is-muted">
          <span className="focus-visualizer-placeholder-label">Visualiser unavailable</span>
        </div>
      )}
    </div>
  );
}
