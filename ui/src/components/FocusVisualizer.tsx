import { useEffect, useRef, useState } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import { setSpectrumTap } from "../lib/commands";
import type { VisualizerMode } from "../lib/visualizerMode";
import { audibleTarget, pickSpectrumFrame, spectrumLastPushAt } from "../lib/spectrumRing";
import { VISUALIZER_PARAMS } from "../lib/visualizerParams";
import {
  RidgeHistory,
  applyGrain,
  drawRidgeline,
  edgeWindow,
  mixBandsInto,
  rerollGrain,
  resampleRow,
  smoothRow,
  spreadRow,
} from "../lib/ridgeline";
import { accentFromPalette } from "../lib/vibrantColor";
import { currentAccent, DEFAULT_ACCENT } from "../lib/accent";

/**
 * Focus-mode live spectrum visualiser.
 *
 * Data source: the `spectrum-frames` event, fed by a libavfilter tap in
 * mpv's `--af` chain (`ramus-core/src/playback/spectrum_tap.rs`). Mounting
 * installs the tap via `setSpectrumTap(true)`; unmounting removes it, as
 * does the off mode, so its CPU cost is only paid while the visualiser is
 * on screen. Frames land in `lib/spectrumRing.ts` keyed by track
 * position, up to ~0.5 s ahead of the reported playback position.
 *
 * Sync: each paint estimates where the audio is from the last audible
 * tick (`playback-audible`: mpv's `audio-pts` with its stream epoch) plus
 * the wall-clock elapsed since it, and draws the ring frame of that
 * epoch nearest the estimate. Because the frames carry mpv's own
 * timestamps and epoch, seeks, pauses, stalls and track changes need no
 * special handling here: no frame near the estimate means no bars, and
 * a gapless join keeps drawing the outgoing track until it is heard.
 *
 * Rendering has two looks, chosen by the `mode` prop
 * (`lib/visualizerMode.ts`; off draws nothing):
 *
 * `bars`: one bar per band per channel hanging from the top edge. A frame
 * carries the left channel's N bands followed by the right channel's (N
 * is 64 by default), drawn as a stereo mirror: the left channel on the
 * left half with its bass at the centre, the right channel on the right
 * half likewise, so treble sits at both edges. No interpolation and no
 * synthetic jitter: every bar is a measured band, and the two halves
 * differ exactly as much as the mix does.
 *
 *     bar 0      → left  band N-1 (highest, far left)
 *     bar N-1    → left  band 0   (lowest, just left of centre)
 *     bar N      → right band 0   (lowest, just right of centre)
 *     bar 2N-1   → right band N-1 (highest, far right)
 *
 * `ridge`: the two channels averaged into N points, bass on the left and
 * treble on the right, resampled to `ridgeOversample` points per band
 * along a monotone cubic and textured, then drawn as a stack of lines
 * rising from the bottom edge: the live line in front and, behind it,
 * one row per `ridgeRowMs` of history (`lib/ridgeline.ts`).
 *
 * Both modes shape every point through a level curve and spring-ease it
 * between frames; the ridge has its own curve and easing values because
 * a scrolling line wants slower dynamics than a bar. The point count
 * comes from the frames themselves; buffers are sized on the first frame
 * and resized if it ever changes.
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
 * Ridge lines are white whatever the accent: the look is ink over the
 * backdrop, and the backdrop already carries the accent.
 */
const RIDGE_RGB = "255, 255, 255";

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
 * Copy one stereo frame's band levels onto the bars in `out` as 0..1,
 * one bar per value: the left channel's bands reversed onto the left
 * half so its bass sits at the centre, the right channel's bands in
 * order onto the right half. `out` must be exactly as long as the frame.
 */
function readBandsInto(bands: Uint8Array, out: Float32Array): void {
  const total = bands.length;
  const n = total / CHANNELS;
  if (out.length !== total || !Number.isInteger(n)) {
    out.fill(0);
    return;
  }
  for (let i = 0; i < n; i++) {
    out[i] = bands[n - 1 - i] / 255;
    out[n + i] = bands[n + i] / 255;
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
/**
 * `performance.now()` when the document last became visible. The paint
 * loop's no-frames hint must not count a hidden stretch, when the tap was
 * deliberately removed.
 */
let tapVisibleSince = 0;
function requestTap(enabled: boolean): void {
  tapRequests = tapRequests
    .then(() => setSpectrumTap(enabled))
    .catch((e) => console.warn(`[spectrum] tap ${enabled ? "install" : "remove"} failed:`, e));
}

interface Props {
  /** Which look to paint; off paints nothing and removes the tap. */
  mode: VisualizerMode;
}

export default function FocusVisualizer({ mode }: Props) {
  const disabled = useSettingsStore((s) => s.disableSpectrum);
  const active = !disabled && mode !== "off";

  // The tap is installed for exactly as long as this component is mounted
  // with a look to paint AND the document is visible: the paint loop
  // stops while the window is hidden, so frames measured then would cost
  // the tap's CPU for nothing. Switching off, or disabling the visualiser
  // in settings, while mounted runs the cleanup, which removes the tap;
  // the backend applies the settings veto on its side too, so a stale
  // install request can't slip through. Switching between the two looks
  // keeps it installed. Every request goes through the chain, so a
  // hide/show pair lands in order like any other toggle.
  useEffect(() => {
    if (!active) return;
    const sync = () => {
      if (!document.hidden) tapVisibleSince = performance.now();
      requestTap(!document.hidden);
    };
    sync();
    document.addEventListener("visibilitychange", sync);
    return () => {
      document.removeEventListener("visibilitychange", sync);
      requestTap(false);
    };
  }, [active]);

  if (!active) return null;

  // Keyed on the mode so a switch remounts the canvas with fresh buffers
  // (the two looks size their point buffers differently) while the tap,
  // owned above, stays installed.
  return <CanvasLayer key={mode} mode={mode} />;
}

// --- Canvas layer ---

function CanvasLayer({ mode }: { mode: Exclude<VisualizerMode, "off"> }) {
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
      // No palette for this track (a same-album skip re-uses the art, so
      // nothing re-extracts, and the cached palette may be absent): keep
      // painting whatever accent the rest of the UI is showing. Before
      // any accent has been applied, match the CSS `:root` defaults.
      const [r, g, b] = currentAccent() ?? [120, 90, 220];
      accentRef.current = { r: String(r), g: String(g), b: String(b) };
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
    // when moving between displays (e.g. 1x to 2x Retina). The backing
    // store is sized in whole device pixels and the CSS box derived from
    // it, so the bitmap maps onto the screen one to one: at a fractional
    // scale (Windows at 125 % or 150 %) `width * dpr` is fractional, the
    // canvas truncates it, and the browser then resamples the bitmap
    // into a box a fraction of a pixel larger, smearing every hairline
    // over two pixel rows with a different blend on each row. Sizing the
    // backing store wipes the canvas; `wiped` tells the next paint so a
    // frame that would otherwise leave the canvas as it is repaints.
    let lastBw = 0;
    let lastBh = 0;
    let lastDpr = 0;
    let wiped = false;

    const resize = () => {
      const rect = container.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const bw = Math.max(1, Math.floor(rect.width * dpr));
      const bh = Math.max(1, Math.floor(rect.height * dpr));
      const w = bw / dpr;
      const h = bh / dpr;
      if (bw !== lastBw || bh !== lastBh || dpr !== lastDpr) {
        canvas.width = bw;
        canvas.height = bh;
        canvas.style.width = `${w}px`;
        canvas.style.height = `${h}px`;
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        lastBw = bw;
        lastBh = bh;
        lastDpr = dpr;
        wiped = true;
      }
      return { w, h };
    };

    const resizeObs = new ResizeObserver(() => resize());
    resizeObs.observe(container);

    // The ridge reads one point per band (both channels averaged); the
    // bars read one per band per channel.
    const ridge = mode === "ridge";
    const pointsFor = (frameWidth: number) => (ridge ? frameWidth / CHANNELS : frameWidth);

    // Per-point buffers. `current` is the eased level each point is drawn
    // at (so it decays smoothly between frames rather than snapping),
    // `scratch` receives the target frame, `shaped` the target after the
    // level curve, and `spread` and `smoothed` (ridge only) the widened
    // and neighbour-blended copies of it. All are reallocated, and the
    // eased levels start from silence, if the point count ever changes.
    let current = new Float32Array(pointsFor(CHANNELS * DEFAULT_BAND_COUNT));
    let scratch = new Float32Array(current.length);
    let shaped = new Float32Array(current.length);
    let spread = new Float32Array(ridge ? current.length : 0);
    let smoothed = new Float32Array(ridge ? current.length : 0);

    // Ridge only, all on the fine grid the eased row is resampled to:
    // the resampled row, the history rows behind it, the per-point edge
    // window, when the last history row was cut, and the grain: one
    // noise value per fine point, applied to the live line as drawn and
    // frozen into each history row when it is cut, then re-rolled, so
    // every row carries its own texture. Rebuilt when a tuning value or
    // the point count changes.
    let fine = new Float32Array(0);
    let history: RidgeHistory | null = null;
    let taper: Float32Array | null = null;
    let taperAmount = NaN;
    let lastRowAt = 0;
    let grain = new Float32Array(0);
    let grained = new Float32Array(0);
    // Wall-clock since the live line has been flat (0 while it isn't),
    // and the row count of the last paint: together they decide when the
    // picture has stopped changing and the paint can be skipped.
    let quietSince = 0;
    let paintedRows = 0;

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
      const repaintForced = wiped;
      wiped = false;

      // Frame delta for time-normalised easing.
      const now = performance.now();
      const rawDelta = lastTs !== 0 ? now - lastTs : 0;
      lastTs = now;

      // Hot-path reads. Playback state gates the lookup and the seek-bar
      // position is its fallback clock — never subscribe via a React
      // selector.
      const playback = usePlaybackStore.getState();
      const isPlaying = playback.status === "playing";

      let haveFrame = false;
      if (isPlaying) {
        // Clock off the audible position (mpv's `audio-pts`, with its
        // stream epoch), extrapolated from the last tick; ticks land
        // several times a second, which is far too coarse on its own.
        // Before the first tick, fall back to the seek-bar position.
        const target = audibleTarget(now);
        const frame = target
          ? pickSpectrumFrame(
              target.epoch,
              target.pos,
              FRAME_LAG_TOLERANCE_S,
              FRAME_LEAD_TOLERANCE_S,
            )
          : pickSpectrumFrame(
              null,
              playback.position + (now - playback.positionAt) / 1000,
              FRAME_LAG_TOLERANCE_S,
              FRAME_LEAD_TOLERANCE_S,
            );
        if (frame) {
          const points = pointsFor(frame.bands.length);
          if (points !== current.length) {
            current = new Float32Array(points);
            scratch = new Float32Array(points);
            shaped = new Float32Array(points);
            spread = new Float32Array(ridge ? points : 0);
            smoothed = new Float32Array(ridge ? points : 0);
          }
          if (ridge) mixBandsInto(frame.bands, CHANNELS, scratch);
          else readBandsInto(frame.bands, scratch);
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
        now - Math.max(spectrumLastPushAt(), mountedAt, tapVisibleSince) > NO_FRAMES_HINT_MS;
      if (starvedNow !== starvedRef.current) {
        starvedRef.current = starvedNow;
        setStarved(starvedNow);
      }

      // Shape each point's level, then spring-ease toward it. The backend
      // delivers dB position through its own compression curve; the
      // floor cut, gamma and gain here are the display-side adjustments
      // (see lib/visualizerParams.ts), one set per mode. Easing alphas
      // are computed once per frame, not per point.
      const P = VISUALIZER_PARAMS;
      const floorCut = ridge ? P.ridgeFloorCut : P.floorCut;
      const gamma = ridge ? P.ridgeGamma : P.gamma;
      const gain = ridge ? P.ridgeGain : P.gain;
      const easeAttack = ridge ? P.ridgeAttack : P.easeAttack;
      const easeDecay = ridge ? P.ridgeDecay : P.easeDecay;
      const shape = floorCut > 0 || gamma !== 1 || gain !== 1;
      const pointCount = current.length;
      for (let i = 0; i < pointCount; i++) {
        let level = scratch[i];
        if (shape) {
          level = level <= floorCut ? 0 : (level - floorCut) / (1 - floorCut);
          if (gamma !== 1) level = Math.pow(level, gamma);
          level = Math.min(1, level * gain);
        }
        shaped[i] = level;
      }
      // The ridge widens each peak and then blends each point with its
      // neighbours, both after the curve: a floor cut leaves one band
      // standing alone as a needle, and the spread turns it back into a
      // peak while the blend softens the hard zeros around it.
      let source = shaped;
      if (ridge && P.ridgeSpread > 0) {
        spreadRow(source, spread, P.ridgeSpread);
        source = spread;
      }
      if (ridge && P.ridgeSmooth > 0) {
        smoothRow(source, smoothed, P.ridgeSmooth);
        source = smoothed;
      }
      const easeDt = Math.min(rawDelta, EASE_DT_CLAMP_MS);
      const dtRatio = easeDt / EASE_REFERENCE_DT_MS;
      const alphaAttack = easeDt > 0 ? 1 - Math.pow(1 - easeAttack, dtRatio) : 0;
      const alphaDecay = easeDt > 0 ? 1 - Math.pow(1 - easeDecay, dtRatio) : 0;
      for (let i = 0; i < pointCount; i++) {
        const prev = current[i];
        const level = source[i];
        const alpha = level > prev ? alphaAttack : alphaDecay;
        current[i] = prev + (level - prev) * alpha;
      }

      if (ridge) {
        const rows = Math.max(1, Math.round(P.ridgeRows));
        const historyRows = Math.max(1, rows - 1);
        const perBand = Math.min(8, Math.max(1, Math.round(P.ridgeOversample)));

        // The live line is flat once its tallest point would rise under
        // half a pixel on the tallest row. Flat for longer than the stack
        // takes to carry a row off the top means every history row is
        // flat too, so the picture is a fixed set of rules and repainting
        // it is wasted work: the canvas keeps its last paint until a
        // frame lifts the line again. A wiped canvas or a changed row
        // count still gets one paint.
        let peak = 0;
        for (let i = 0; i < pointCount; i++) if (current[i] > peak) peak = current[i];
        const rise = h * P.ridgePeak * Math.max(1, P.ridgeDepthScale);
        if (peak * rise >= MIN_VISIBLE_HEIGHT_PX) quietSince = 0;
        else if (quietSince === 0) quietSince = now;
        const settled = quietSince !== 0 && now - quietSince > (rows + 1) * P.ridgeRowMs;

        if (!settled || repaintForced || rows !== paintedRows) {
          ctx.clearRect(0, 0, w, h);
          paintedRows = rows;
          // One point has no interval to resample; the row count and the
          // fine grid both come out of the intervals between points.
          if (pointCount >= 2) {
            const fineCount = (pointCount - 1) * perBand + 1;
            if (fine.length !== fineCount) {
              fine = new Float32Array(fineCount);
              grain = new Float32Array(fineCount);
              grained = new Float32Array(fineCount);
              rerollGrain(grain);
            }
            if (!history || history.rows !== historyRows || history.width !== fineCount) {
              history = new RidgeHistory(historyRows, fineCount);
            }
            if (!taper || taper.length !== fineCount || taperAmount !== P.ridgeEdgeTaper) {
              taper = edgeWindow(fineCount, P.ridgeEdgeTaper);
              taperAmount = P.ridgeEdgeTaper;
            }
            resampleRow(current, fine, perBand);
            // A history row is cut from the live line every `ridgeRowMs`
            // of wall-clock, so the stack scrolls at one speed whatever
            // the display's refresh rate. It keeps scrolling through a
            // pause, carrying the flat line up until every row is flat;
            // the rows never go away, a flat row is a rule at its
            // baseline. The cut time steps by the period so the cadence
            // keeps its fractional credit rather than rounding up to the
            // frame rate; after a hitch longer than two periods it
            // resyncs instead of replaying the gap as a burst of rows.
            if (now - lastRowAt >= P.ridgeRowMs) {
              applyGrain(fine, grained, grain, P.ridgeGrain);
              history.push(grained);
              rerollGrain(grain);
              lastRowAt = now - lastRowAt > 2 * P.ridgeRowMs ? now : lastRowAt + P.ridgeRowMs;
            }
            applyGrain(fine, grained, grain, P.ridgeGrain);
            const behind = history;
            const live = grained;
            drawRidgeline(
              ctx,
              w,
              h,
              rows,
              (r) => (r === 0 ? live : behind.get(r - 1)),
              taper,
              RIDGE_RGB,
              P,
            );
          }
        }
      } else {
        ctx.clearRect(0, 0, w, h);
        // Accent colour from the cached ref; no per-frame style-recalc.
        const { r, g, b } = accentRef.current;
        const { barAlpha, barMaxHeight, barTipOpacity, barGap, barSpan } = P;

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
        const totalGap = barGap * (pointCount - 1);
        const barWidth = Math.max(0.5, (fieldW - totalGap) / pointCount);

        for (let i = 0; i < pointCount; i++) {
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
      }

      rafRef.current = requestAnimationFrame(render);
    };

    rafRef.current = requestAnimationFrame(render);

    return () => {
      cancelAnimationFrame(rafRef.current);
      resizeObs.disconnect();
    };
  }, [mode]);

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
