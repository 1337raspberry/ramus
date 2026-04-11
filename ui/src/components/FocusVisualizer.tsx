import { useEffect, useMemo, useRef } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { spectrumKind, type SpectrumFrames, type SpectrumState } from "../lib/types";
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
 * Three visual states:
 *   - "analysing"   → centred "Analysing audio…" placeholder
 *   - "unavailable" → centred "Visualiser not available while transcoding"
 *                     (or whichever `reason` the backend supplies)
 *   - ready         → 256 mirrored bars across the top of the focus view
 *
 * Rendering model: 128 FFT bands are rendered **twice** — once per half
 * of the canvas, mirrored about the vertical centreline. Bass sits in
 * the middle and treble at both outer edges, producing a symmetric
 * "mountain" that pulses with low frequencies and shimmers at the wings.
 *
 *   bar index 0  → band 127 (highest treble, far left)
 *   bar index 127→ band 0   (lowest bass, just left of centre)
 *   bar index 128→ band 0   (lowest bass, just right of centre)
 *   bar index 255→ band 127 (highest treble, far right)
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
  // memo resolved it to non-null for the "ready" kind.
  return <BarsLayer frames={normalisedFrames!} />;
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

// --- Bars layer ---

function BarsLayer({ frames }: { frames: SpectrumFrames }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number>(0);

  // Persistent per-bar eased height so bars decay smoothly between
  // frames rather than snapping. Float32 for cheap lerping.
  const currentRef = useRef<Float32Array>(new Float32Array(BAR_COUNT));
  // Reusable scratch buffer so the RAF loop doesn't allocate every frame.
  const scratchRef = useRef<Uint8Array>(new Uint8Array(BAR_COUNT));

  // Bass-decorrelation noise tables — one value per *band* (not per
  // bar) so left/right mirror bars using the same band stay in sync
  // and the overall mirror symmetry is preserved. Random phase +
  // slow-ish frequency per band means adjacent bands wobble out of
  // phase with each other, which breaks the lockstep on sub-bin
  // bands that all read the same FFT bin. See the draw loop below
  // for the gating logic, and the physics explanation is in the
  // `bassNoiseAmount` doc in focusVizDebugStore.ts.
  const noisePhasesRef = useRef<Float32Array | null>(null);
  const noiseFreqsRef = useRef<Float32Array | null>(null);

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

      // Accent colour from CSS variables so bars match the album art.
      const styles = getComputedStyle(document.documentElement);
      const r = styles.getPropertyValue("--accent-r").trim() || "120";
      const g = styles.getPropertyValue("--accent-g").trim() || "90";
      const b = styles.getPropertyValue("--accent-b").trim() || "220";

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
      // share the same FFT bin don't move in identical lockstep. Gated
      // on the current amplitude so silent bars stay perfectly still —
      // noise on near-zero bars would look like flickery bottom pixels.
      //
      // Applied per BAND (not per bar) so the left and right mirror
      // halves stay in sync — band N's wobble is identical on both
      // sides of centre, preserving the mirror symmetry while still
      // decorrelating band N from band N±1. See the useEffect init
      // block above for the phase/frequency table.
      const noisePhases = noisePhasesRef.current;
      const noiseFreqs = noiseFreqsRef.current;
      const noiseActive = viz.bassNoiseAmount > 0 && viz.bassNoiseBandCutoff > 0;
      const noiseT = noiseActive ? performance.now() / 1000 : 0;

      for (let i = 0; i < BAR_COUNT; i++) {
        let h = current[i];

        if (noiseActive && noisePhases && noiseFreqs) {
          const band = barToBand(i);
          if (band < viz.bassNoiseBandCutoff && h > viz.bassNoiseGate) {
            const wobble = Math.sin(noiseT * noiseFreqs[band] + noisePhases[band]);
            h = h * (1 + wobble * viz.bassNoiseAmount);
            if (h < 0) h = 0;
            else if (h > 1) h = 1;
          }
        }

        const barHeight = h * maxH;
        // DEBUG (focus viz tuning): MIN_VISIBLE_HEIGHT_PX
        if (barHeight < viz.minVisibleHeightPx) continue;
        const x = i * (barWidth + viz.barGapPx);
        ctx.fillRect(x, 0, barWidth, barHeight);
        if (drawBorder) {
          ctx.strokeRect(x, 0, barWidth, barHeight);
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
