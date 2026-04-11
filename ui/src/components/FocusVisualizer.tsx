import { useEffect, useMemo, useRef } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { spectrumKind, type SpectrumFrames, type SpectrumState } from "../lib/types";
import { accentFromPalette } from "../lib/vibrantColor";
// DEBUG (focus viz tuning): runtime-tunable parameters via the slider panel.
// Remove this import + every line tagged `DEBUG (focus viz tuning)` to
// rip out the debug system. See stores/focusVizDebugStore.ts for the
// full removal checklist.
import { useFocusVizDebugStore } from "../stores/focusVizDebugStore";

/**
 * Focus-mode FFT visualiser — 128-band mirrored spectrogram.
 *
 * Data source: `spectrumState` in `playbackStore`, populated from the
 * `get_spectrum` Tauri command (per-track precomputed spectrograms from
 * symphonia + realfft in Rust). No live audio metering — the old astats
 * pulse path is gone. Sync is automatic because we look up frames by
 * `floor(positionMs / hopMs)` using mpv's own `time-pos`, so bars align
 * sample-accurately with whatever the speakers are playing.
 *
 * Placeholder states (shown before the spectrogram is ready):
 *   - "analysing"   → "Analysing audio…" label
 *   - "unavailable" → "Visualiser not available while transcoding" (or
 *                     whatever `reason` the backend supplies)
 *
 * Ready-state rendering modes, cycled by the wave-icon button in
 * FocusNowPlayingView's track row (`playbackStore.cycleVisualizerMode`):
 *
 *   - "bars" → 256 mirrored bars across the top. 128 FFT bands are
 *     rendered **twice** (once per half of the canvas, mirrored about
 *     the vertical centreline), so bass sits in the middle and treble
 *     at both outer edges, producing a symmetric mountain silhouette.
 *
 *       bar index 0  → band 127 (highest treble, far left)
 *       bar index 127→ band 0   (lowest bass, just left of centre)
 *       bar index 128→ band 0   (lowest bass, just right of centre)
 *       bar index 255→ band 127 (highest treble, far right)
 *
 *   - "line" → a single smoothed curve across the full width, filled
 *     from the top edge down. Uses the same underlying eased bar
 *     heights but passes them through a moving-average window and
 *     draws a filled path with quadratic curves between points for a
 *     glassy organic look. Smoothing window is tuned via the debug
 *     slider (`lineSmoothingWindow`).
 *
 * Mode switches don't remount the canvas — the draw loop reads
 * `playbackStore.visualizerMode` via `getState()` each frame and
 * branches. This keeps the eased-height buffer continuous across
 * bars↔line transitions so there's no flicker.
 */

const BAR_COUNT = 256; // 128 bands × 2 mirrored halves
const HALF_BAR_COUNT = BAR_COUNT / 2;

// DEBUG (focus viz tuning): the constants that used to live here have
// been moved into `focusVizDebugStore.ts` so the slider panel can tune
// them live. When you remove the debug system, restore them here.
// These are the current baked defaults (kept in sync with
// FOCUS_VIZ_DEFAULTS in focusVizDebugStore.ts):
//
//   const BAR_MAX_HEIGHT_PCT = 20;       // top 20% of the focus window
//   const BAR_GAP_PX = 1;                // px between adjacent bars
//   const MIN_VISIBLE_HEIGHT_PX = 0.5;   // skip-draw threshold
//   const EASE_ATTACK = 0.55;            // snappy rise on transients
//   const EASE_DECAY = 0.35;             // slower fall to filter jitter
//   const GRADIENT_TOP_OPACITY = 0.95;   // alpha at the top of bars
//   const GRADIENT_BOTTOM_OPACITY = 0.3; // alpha at the bottom tips
//   const GLOBAL_ALPHA = 0.3;            // ctx.globalAlpha multiplier
//   const BORDER_WIDTH_PX = 0;           // no border by default
//   const BORDER_OPACITY = 0;
//   const PLACEHOLDER_TOP_VH = 1.5;      // "Analysing audio…" offset
//   const BASS_NOISE_AMOUNT = 0.08;      // ±8% wobble on bass bars
//   const BASS_NOISE_BAND_CUTOFF = 40;   // apply to bands 0..39
//   const BASS_NOISE_GATE = 0.08;        // skip bars below 8% height
//   const LINE_SMOOTHING_WINDOW = 12;    // moving-average window (line mode)
//
// (or whichever values you've settled on after tuning — check the
// `focus-viz-debug` localStorage key in DevTools to see them.)

/** Map a canvas bar index (0..255) to its source band index (0..127). */
function barToBand(barIndex: number): number {
  if (barIndex < HALF_BAR_COUNT) {
    // Left half: 0 = highest band, HALF_BAR_COUNT-1 = lowest band.
    return HALF_BAR_COUNT - 1 - barIndex;
  }
  // Right half: HALF_BAR_COUNT = lowest band, BAR_COUNT-1 = highest band.
  return barIndex - HALF_BAR_COUNT;
}

/**
 * Read bar heights for the current frame into `out` (a Uint8Array of
 * length BAR_COUNT). Returns true if a frame was available, false if
 * we're past the end of the spectrogram (in which case the caller
 * decays bars toward zero).
 */
function readFrameInto(frames: SpectrumFrames, positionMs: number, out: Uint8Array): boolean {
  const bands = frames.bandCount;
  if (bands === 0 || frames.hopMs <= 0) {
    out.fill(0);
    return false;
  }

  const frameIdx = Math.floor(positionMs / frames.hopMs);
  const rawFrames = frames.frames;
  const totalFrames = Math.floor(rawFrames.length / bands);
  if (frameIdx < 0 || frameIdx >= totalFrames) {
    out.fill(0);
    return false;
  }

  const start = frameIdx * bands;
  // Read through the BAR_COUNT canvas bars via the mirror map.
  for (let i = 0; i < BAR_COUNT; i++) {
    const band = barToBand(i);
    // Guard against a spectrogram with < 128 bands (custom config).
    out[i] = band < bands ? (rawFrames as ArrayLike<number>)[start + band] : 0;
  }
  return true;
}

/**
 * Convert a serde-serialised `SpectrumFrames` (which arrives as either
 * a plain number[] or a Uint8Array over Tauri IPC) into a view that
 * guarantees constant-time indexed reads. We normalise to Uint8Array
 * once per track change so the RAF hot path never has to type-check.
 */
function normaliseFrames(frames: SpectrumFrames): SpectrumFrames {
  if (frames.frames instanceof Uint8Array) return frames;
  return { ...frames, frames: Uint8Array.from(frames.frames) };
}

export default function FocusVisualizer() {
  // Subscribe to the TOP-LEVEL spectrum state so we can pick between
  // the canvas and placeholder render. The canvas itself reads position
  // via getState() inside the RAF loop — that's still the hot path.
  const spectrumState = usePlaybackStore((s) => s.spectrumState);

  // Cache the normalised frames for the currently-ready state so the
  // RAF loop can index into Uint8Array directly without re-normalising
  // every frame. `useMemo` is keyed on the identity of the underlying
  // frames — set() creates a new object on every track change, so this
  // recomputes exactly when it should.
  const normalisedFrames = useMemo<SpectrumFrames | null>(() => {
    if (typeof spectrumState === "object" && spectrumState !== null && "ready" in spectrumState) {
      return normaliseFrames(spectrumState.ready);
    }
    return null;
  }, [spectrumState]);

  const kind = spectrumKind(spectrumState ?? "analysing");

  // When we're showing a placeholder there's no need to drive an RAF
  // loop at all — render plain React. The ready branch below owns its
  // own canvas + RAF that we tear down on unmount / state change.
  if (kind !== "ready") {
    return <PlaceholderLayer state={spectrumState} />;
  }

  // Ready path: render the canvas layer. Pull `normalisedFrames!` — the
  // memo resolved it to non-null for the "ready" kind. The layer reads
  // `visualizerMode` via getState() each frame so mode cycling doesn't
  // remount the canvas.
  return <CanvasLayer frames={normalisedFrames!} />;
}

// --- Placeholder layer ---

function PlaceholderLayer({ state }: { state: SpectrumState | null }) {
  // DEBUG (focus viz tuning): vertical position is store-driven so the
  // panel slider can move the label without re-render churn. When the
  // debug system is removed, drop this selector and let the CSS
  // `padding-top` value (also tagged DEBUG in styles.css) take over.
  const placeholderTopVh = useFocusVizDebugStore((s) => s.placeholderTopVh);

  let label: string;
  let muted = false;
  if (state === null || state === "analysing") {
    label = "Analysing audio…";
  } else if ("unavailable" in state) {
    const reason = state.unavailable.reason;
    if (reason === "transcoding") {
      label = "Visualiser unavailable while transcoding";
    } else if (reason === "file_missing") {
      label = "Analysing audio…";
    } else if (reason.startsWith("unsupported_codec")) {
      label = "Visualiser unavailable for this codec";
    } else {
      label = "Visualiser unavailable";
    }
    muted = true;
  } else {
    label = "Analysing audio…";
  }

  return (
    <div
      className={`focus-visualizer focus-visualizer-placeholder${muted ? " is-muted" : ""}`}
      // DEBUG (focus viz tuning): inline style overrides CSS padding-top.
      style={{ paddingTop: `${placeholderTopVh}vh` }}
    >
      <span className="focus-visualizer-placeholder-label">{label}</span>
    </div>
  );
}

// --- Canvas layer (bars + line modes share one canvas + RAF) ---

/**
 * In-place moving-average smoothing over `src[]` with a symmetric
 * window of `windowSize` samples. Writes into `out[]`, which must have
 * the same length as `src`. O(n·windowSize) per call — fine for
 * n=256, window<=40 at 60 fps. Used by the line-mode draw path to
 * turn the jagged per-band eased heights into a continuous curve.
 */
function smoothLineInto(src: Float32Array, windowSize: number, out: Float32Array): void {
  const n = src.length;
  const halfWindow = Math.floor(Math.max(1, windowSize) / 2);
  for (let i = 0; i < n; i++) {
    const lo = Math.max(0, i - halfWindow);
    const hi = Math.min(n, i + halfWindow + 1);
    let sum = 0;
    for (let j = lo; j < hi; j++) sum += src[j];
    out[i] = sum / (hi - lo);
  }
}

function CanvasLayer({ frames }: { frames: SpectrumFrames }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number>(0);

  // Persistent per-bar eased height so bars decay smoothly between
  // frames rather than snapping. Float32 for cheap lerping.
  const currentRef = useRef<Float32Array>(new Float32Array(BAR_COUNT));
  // Reusable scratch buffer so the RAF loop doesn't allocate every frame.
  const scratchRef = useRef<Uint8Array>(new Uint8Array(BAR_COUNT));
  // Line-mode smoothing output buffer, reused across frames so the
  // moving-average pass doesn't allocate every frame.
  const smoothedLineRef = useRef<Float32Array>(new Float32Array(BAR_COUNT));

  // Bass-decorrelation noise tables — one value per *band* (not per
  // bar) so left/right mirror bars using the same band stay in sync
  // and the overall mirror symmetry is preserved. Random phase +
  // slow-ish frequency per band means adjacent bands wobble out of
  // phase with each other, which breaks the lockstep on sub-bin
  // bands that all read the same FFT bin. See the draw loop below
  // for the gating logic, and the physics explanation is in the
  // `bassNoiseAmount` doc in focusVizDebugStore.ts.
  // Only used by the "bars" draw branch — the "line" branch's
  // horizontal smoothing averages the lockstep out on its own.
  const noisePhasesRef = useRef<Float32Array | null>(null);
  const noiseFreqsRef = useRef<Float32Array | null>(null);

  // Cached accent RGB (as pre-stringified channel values ready for
  // rgba() template literals). Refreshed via the effect below whenever
  // the vibrantPalette in playbackStore changes, which is at most once
  // per track change. The RAF loop reads from this ref every frame
  // instead of calling `getComputedStyle(document.documentElement)`,
  // which would otherwise trigger a browser style-recalc query 60× a
  // second to probe a value that barely ever changes.
  const accentRef = useRef<{ r: string; g: string; b: string }>({
    r: "120",
    g: "90",
    b: "220",
  });
  const vibrantPalette = usePlaybackStore((s) => s.vibrantPalette);
  useEffect(() => {
    if (!vibrantPalette) {
      // No palette yet (fresh session, or a track with no art).
      // Fall back to the same defaults the CSS `:root` rule ships.
      accentRef.current = { r: "120", g: "90", b: "220" };
      return;
    }
    const [r, g, b] = accentFromPalette(vibrantPalette);
    accentRef.current = { r: String(r), g: String(g), b: String(b) };
  }, [vibrantPalette]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // Track DPR and dimensions so the backing store resizes cleanly
    // across displays (e.g. window drag from 1x to 2x Retina).
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

    // Reset eased-height buffer on track change so the old song's tail
    // doesn't bleed into the new song's intro.
    currentRef.current.fill(0);

    // Initialise per-band noise tables on first mount. Re-using across
    // track changes is fine (and desirable — we don't want the wobble
    // pattern to jump every time the track changes).
    if (!noisePhasesRef.current || !noiseFreqsRef.current) {
      const phases = new Float32Array(HALF_BAR_COUNT); // one per band
      const freqs = new Float32Array(HALF_BAR_COUNT);
      for (let i = 0; i < HALF_BAR_COUNT; i++) {
        phases[i] = Math.random() * Math.PI * 2;
        // 0.6 – 2.4 Hz slow drift — fast enough to visibly decorrelate
        // adjacent bands, slow enough to look like natural resonance
        // rather than jitter.
        freqs[i] = 0.6 + Math.random() * 1.8;
      }
      noisePhasesRef.current = phases;
      noiseFreqsRef.current = freqs;
    }

    const render = () => {
      const { w, h } = resize();
      ctx.clearRect(0, 0, w, h);

      // Hot-path reads — position is the ground truth for where in the
      // spectrogram we are. Never subscribe to this via a selector.
      const playback = usePlaybackStore.getState();
      const isPlaying = playback.status === "playing";
      const positionMs = playback.position * 1000;
      // Mode is read every frame (not captured in closure) so cycling
      // bars ↔ line via the wave button takes effect on the next paint
      // without remounting the canvas / resetting the eased buffer.
      const mode = playback.visualizerMode;

      // DEBUG (focus viz tuning): pull all tunable params via getState()
      // each frame so slider tweaks take effect on the very next paint.
      // When removing the debug system, replace each `viz.X` reference
      // below with the constant from the block at the top of this file.
      const viz = useFocusVizDebugStore.getState();

      const scratch = scratchRef.current;
      if (isPlaying) {
        readFrameInto(frames, positionMs, scratch);
      } else {
        // Paused/stopped: decay toward zero so the bars don't freeze
        // at whatever the last value was.
        scratch.fill(0);
      }

      // Spring-ease each bar toward its target height.
      const current = currentRef.current;
      for (let i = 0; i < BAR_COUNT; i++) {
        const target = scratch[i] / 255;
        const prev = current[i];
        // DEBUG (focus viz tuning): EASE_ATTACK / EASE_DECAY
        const ease = target > prev ? viz.easeAttack : viz.easeDecay;
        current[i] = prev + (target - prev) * ease;
      }

      // Accent colour from the cached ref — updated by the
      // vibrantPalette effect above, no per-frame style-recalc cost.
      const { r, g, b } = accentRef.current;

      // DEBUG (focus viz tuning): GLOBAL_ALPHA
      ctx.globalAlpha = viz.globalAlpha;

      // Gradient fill: opaque near the top edge (where the bars
      // originate), fading to transparent at their bottom tips. Creates
      // a cohesive glow that doesn't compete with the track title below.
      // DEBUG (focus viz tuning): BAR_MAX_HEIGHT_PCT
      const maxH = (h * viz.barMaxHeightPct) / 100;
      const grad = ctx.createLinearGradient(0, 0, 0, maxH);
      // DEBUG (focus viz tuning): GRADIENT_TOP_OPACITY / GRADIENT_BOTTOM_OPACITY
      grad.addColorStop(0, `rgba(${r}, ${g}, ${b}, ${viz.gradientTopOpacity})`);
      grad.addColorStop(1, `rgba(${r}, ${g}, ${b}, ${viz.gradientBottomOpacity})`);
      ctx.fillStyle = grad;

      if (mode === "line") {
        // --- Line mode ---
        //
        // Average the eased bar heights with a symmetric moving-average
        // window, then draw a filled curve from the top edge down to
        // the smoothed profile. Using quadratic segments between
        // consecutive points — each actual point becomes a control
        // point, with segment midpoints as on-curve anchors — gives a
        // continuous, glassy curve without needing bezier tension
        // math. The mirror symmetry of the underlying band data is
        // preserved automatically because `current[]` is already mirrored
        // (bars 127 and 128 both read band 0, etc).
        //
        // Noise decorrelation is deliberately skipped in this mode —
        // the horizontal smoothing averages the bass lockstep out on
        // its own, and adding per-band wobble on top would just add
        // high-frequency ripple to an otherwise smooth curve.
        //
        // DEBUG (focus viz tuning): LINE_SMOOTHING_WINDOW
        const smoothed = smoothedLineRef.current;
        smoothLineInto(current, viz.lineSmoothingWindow, smoothed);

        const n = BAR_COUNT;
        const xStep = n > 1 ? w / (n - 1) : w;

        // Path construction: start from the top-left corner, thread a
        // quadratic chain through the smoothed heights, close via a
        // mirror quad to the top-right corner, then `closePath` back
        // across the top edge. The two closing quads are shaped so
        // their tangents at (0,0) and (w,0) are VERTICAL, which
        // matches the implicit top-edge closure perfectly — no kinks
        // at either edge.
        //
        // The midpoint-quadratic technique in the loop (each data
        // point becomes a control, segment midpoints are the on-curve
        // anchors) keeps the interior continuous for free.
        ctx.beginPath();
        ctx.moveTo(0, 0);
        // Left entry quad: tangent at (0,0) points toward the control
        // (0, bar[0]*maxH) = straight down, matching the top edge's
        // implicit vertical closure. The anchor at the first segment
        // midpoint hands off cleanly to the loop below.
        for (let i = 0; i < n - 1; i++) {
          const x1 = i * xStep;
          const y1 = smoothed[i] * maxH;
          const x2 = (i + 1) * xStep;
          const y2 = smoothed[i + 1] * maxH;
          ctx.quadraticCurveTo(x1, y1, (x1 + x2) / 2, (y1 + y2) / 2);
        }
        // Right exit quad: mirror of the entry. Control at
        // (w, bar[n-1]*maxH), anchor at (w, 0). Tangent at (w, 0) is
        // vertical, matching the top edge on the other side.
        ctx.quadraticCurveTo(w, smoothed[n - 1] * maxH, w, 0);
        ctx.closePath();
        ctx.fill();
      } else {
        // --- Bars mode (default) ---
        //
        // DEBUG (focus viz tuning): BORDER_WIDTH_PX / BORDER_OPACITY.
        // Stroke is applied per-bar after the fill, only when width > 0.
        const drawBorder = viz.borderWidthPx > 0 && viz.borderOpacity > 0;
        if (drawBorder) {
          ctx.lineWidth = viz.borderWidthPx;
          ctx.strokeStyle = `rgba(${r}, ${g}, ${b}, ${viz.borderOpacity})`;
        }

        // Bar layout: evenly spaced across the full width. Width is
        // computed once per frame (resize-aware) from the canvas size.
        // DEBUG (focus viz tuning): BAR_GAP_PX
        const totalGap = viz.barGapPx * (BAR_COUNT - 1);
        const barWidth = Math.max(0.5, (w - totalGap) / BAR_COUNT);

        // Bass decorrelation: for low-frequency bands below the cutoff,
        // apply a tiny per-band sinusoidal wobble so adjacent bars that
        // share the same FFT bin don't move in identical lockstep.
        // Gated on the current amplitude so silent bars stay perfectly
        // still — noise on near-zero bars would look like flickery
        // bottom pixels. Applied per BAND (not per bar) so the left
        // and right mirror halves stay in sync. See the useEffect
        // init block above for the phase/frequency table.
        const noisePhases = noisePhasesRef.current;
        const noiseFreqs = noiseFreqsRef.current;
        const noiseActive = viz.bassNoiseAmount > 0 && viz.bassNoiseBandCutoff > 0;
        const noiseT = noiseActive ? performance.now() / 1000 : 0;

        for (let i = 0; i < BAR_COUNT; i++) {
          let barH = current[i];

          if (noiseActive && noisePhases && noiseFreqs) {
            const band = barToBand(i);
            if (band < viz.bassNoiseBandCutoff && barH > viz.bassNoiseGate) {
              const wobble = Math.sin(noiseT * noiseFreqs[band] + noisePhases[band]);
              barH = barH * (1 + wobble * viz.bassNoiseAmount);
              if (barH < 0) barH = 0;
              else if (barH > 1) barH = 1;
            }
          }

          const barHeight = barH * maxH;
          // DEBUG (focus viz tuning): MIN_VISIBLE_HEIGHT_PX
          if (barHeight < viz.minVisibleHeightPx) continue;
          const x = i * (barWidth + viz.barGapPx);
          ctx.fillRect(x, 0, barWidth, barHeight);
          if (drawBorder) {
            ctx.strokeRect(x, 0, barWidth, barHeight);
          }
        }
      }

      // DEBUG (focus viz tuning): reset globalAlpha so anything else
      // sharing the canvas (none today, but defensive) starts fresh.
      ctx.globalAlpha = 1.0;

      rafRef.current = requestAnimationFrame(render);
    };

    rafRef.current = requestAnimationFrame(render);

    return () => {
      cancelAnimationFrame(rafRef.current);
      resizeObs.disconnect();
    };
  }, [frames]);

  return (
    <div ref={containerRef} className="focus-visualizer">
      <canvas ref={canvasRef} />
    </div>
  );
}
