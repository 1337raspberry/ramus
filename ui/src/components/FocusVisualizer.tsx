import { useEffect, useMemo, useRef } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import { spectrumKind, type SpectrumFrames, type SpectrumState } from "../lib/types";
import { isHDR } from "../lib/hdr";
import { accentFromPalette } from "../lib/vibrantColor";

/**
 * Focus-mode 128-band mirrored FFT visualiser.
 *
 * Data source: `spectrumState` in `playbackStore`, populated from the
 * `get_spectrum` Tauri command (per-track precomputed spectrograms via
 * symphonia + realfft). No live metering. Sync is automatic: frames are
 * indexed by `floor(positionMs / hopMs)` against mpv's `time-pos`, so
 * bars align sample-accurately with the speakers.
 *
 * Placeholder states:
 *   - "analysing"   → "Analysing audio…" label
 *   - "unavailable" → reason-specific label supplied by the backend
 *
 * Ready-state rendering modes (cycled by `cycleVisualizerMode`):
 *
 *   - "bars" → 256 mirrored bars. 128 bands render twice, mirrored
 *     about the centreline, so bass sits in the middle and treble at
 *     both edges.
 *
 *       bar 0   → band 127 (highest treble, far left)
 *       bar 127 → band 0   (lowest bass, just left of centre)
 *       bar 128 → band 0   (lowest bass, just right of centre)
 *       bar 255 → band 127 (highest treble, far right)
 *
 *   - "line" → single smoothed curve across the full width, filled
 *     from the top edge down. Same eased bar heights passed through a
 *     moving-average window, drawn as a quadratic path.
 *
 * Mode switches do not remount the canvas; the draw loop reads
 * `visualizerMode` via `getState()` each frame so the eased-height
 * buffer stays continuous across bars ↔ line transitions.
 */

const BAR_COUNT = 256; // 128 bands * 2 mirrored halves
const HALF_BAR_COUNT = BAR_COUNT / 2;

const BAR_MAX_HEIGHT_PCT = 20;
const BAR_GAP_PX = 1;
const MIN_VISIBLE_HEIGHT_PX = 0.5;
const EASE_ATTACK = 0.55;
const EASE_DECAY = 0.35;
const GRADIENT_TOP_OPACITY = 0.95;
const GRADIENT_BOTTOM_OPACITY = isHDR ? 0.3 : 0.45;
const GLOBAL_ALPHA = isHDR ? 0.3 : 0.45;
const BORDER_WIDTH_PX = 0;
const BORDER_OPACITY = 0;
const BASS_NOISE_AMOUNT = 0.08;
const BASS_NOISE_BAND_CUTOFF = 40;
const BASS_NOISE_GATE = 0.08;
const LINE_SMOOTHING_WINDOW = 12;

/** Map a canvas bar index (0..255) to its source band index (0..127). */
function barToBand(barIndex: number): number {
  if (barIndex < HALF_BAR_COUNT) {
    // Left half: 0 = highest band, HALF_BAR_COUNT - 1 = lowest band.
    return HALF_BAR_COUNT - 1 - barIndex;
  }
  // Right half: HALF_BAR_COUNT = lowest band, BAR_COUNT - 1 = highest.
  return barIndex - HALF_BAR_COUNT;
}

/**
 * Read bar heights for the current frame into `out` (a Uint8Array of
 * length BAR_COUNT). Returns true if a frame was available, false if
 * past the end of the spectrogram (caller should decay toward zero).
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
  for (let i = 0; i < BAR_COUNT; i++) {
    const band = barToBand(i);
    // Guard against a spectrogram with < 128 bands (custom config).
    out[i] = band < bands ? (rawFrames as ArrayLike<number>)[start + band] : 0;
  }
  return true;
}

/**
 * Normalise `SpectrumFrames.frames` to a Uint8Array so the RAF hot path
 * gets constant-time indexed reads without type-checking every frame.
 */
function normaliseFrames(frames: SpectrumFrames): SpectrumFrames {
  if (frames.frames instanceof Uint8Array) return frames;
  return { ...frames, frames: Uint8Array.from(frames.frames) };
}

export default function FocusVisualizer() {
  const disabled = useSettingsStore((s) => s.disableSpectrum);

  // Subscribe to the top-level spectrum state so the canvas vs
  // placeholder choice re-renders. The RAF loop still reads position via
  // getState() on the hot path.
  const spectrumState = usePlaybackStore((s) => s.spectrumState);

  // Cache normalised frames so the RAF loop indexes into Uint8Array
  // directly. Keyed on frames identity — `set` creates a new object per
  // track change, which is exactly when this should recompute.
  const normalisedFrames = useMemo<SpectrumFrames | null>(() => {
    if (typeof spectrumState === "object" && spectrumState !== null && "ready" in spectrumState) {
      return normaliseFrames(spectrumState.ready);
    }
    return null;
  }, [spectrumState]);

  if (disabled) return null;

  const kind = spectrumKind(spectrumState ?? "analysing");

  // Placeholders don't drive an RAF loop. The ready branch owns its own
  // canvas + RAF, torn down on unmount or state change.
  if (kind !== "ready") {
    return <PlaceholderLayer state={spectrumState} />;
  }

  // `normalisedFrames!` is non-null for the "ready" kind. The layer
  // reads `visualizerMode` via getState() each frame so mode cycling
  // does not remount the canvas.
  return <CanvasLayer frames={normalisedFrames!} />;
}

// --- Placeholder layer ---

function PlaceholderLayer({ state }: { state: SpectrumState | null }) {
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
    <div className={`focus-visualizer focus-visualizer-placeholder${muted ? " is-muted" : ""}`}>
      <span className="focus-visualizer-placeholder-label">{label}</span>
    </div>
  );
}

// --- Canvas layer (bars + line modes share one canvas + RAF) ---

/**
 * Symmetric moving-average smoothing over `src`, writing into `out`
 * (same length). O(n*windowSize) per call; fine for n=256, window<=40 at
 * 60 fps. Used by line mode to turn per-band eased heights into a
 * continuous curve.
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
  // Reusable scratch buffer; avoids allocation on the RAF hot path.
  const scratchRef = useRef<Uint8Array>(new Uint8Array(BAR_COUNT));
  // Reused line-mode smoothing output buffer.
  const smoothedLineRef = useRef<Float32Array>(new Float32Array(BAR_COUNT));

  // Bass-decorrelation noise tables, one value per *band* (not per
  // bar) so left/right mirror bars stay in sync and mirror symmetry is
  // preserved. Random phase and slow per-band frequency mean adjacent
  // bands wobble out of phase, breaking lockstep on sub-bin bands that
  // read the same FFT bin. BASS_NOISE_AMOUNT sets peak wobble as a
  // fraction of bar height. Bars mode only; line mode's horizontal
  // smoothing averages the lockstep out.
  const noisePhasesRef = useRef<Float32Array | null>(null);
  const noiseFreqsRef = useRef<Float32Array | null>(null);

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
  useEffect(() => {
    if (!vibrantPalette) {
      // Fresh session or a track with no art. Match the CSS `:root`
      // defaults.
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

    // Reset eased-height buffer on track change so the previous
    // track's tail doesn't bleed into the intro.
    currentRef.current.fill(0);

    // Per-band noise tables initialised once per mount. Reusing across
    // track changes is intentional so the wobble pattern stays stable.
    if (!noisePhasesRef.current || !noiseFreqsRef.current) {
      const phases = new Float32Array(HALF_BAR_COUNT); // one per band
      const freqs = new Float32Array(HALF_BAR_COUNT);
      for (let i = 0; i < HALF_BAR_COUNT; i++) {
        phases[i] = Math.random() * Math.PI * 2;
        // 0.6 – 2.4 Hz: fast enough to decorrelate adjacent bands,
        // slow enough to look like natural resonance rather than
        // jitter.
        freqs[i] = 0.6 + Math.random() * 1.8;
      }
      noisePhasesRef.current = phases;
      noiseFreqsRef.current = freqs;
    }

    // Timestamp of the previous RAF callback; feeds `rawDelta` for
    // time-normalised easing.
    let lastTs = 0;

    // EASE_ATTACK / EASE_DECAY are tuned against a 60Hz reference.
    // Rescale per actual frame dt via
    //   alpha = 1 - (1 - ease) ^ (dt / referenceDt)
    // so wall-clock behaviour matches across 60/120/360 Hz displays.
    // Without this the lerp converges 6x faster on a 360Hz display,
    // collapsing the rise/fall into a snap+plateau pattern
    // synchronised to the 30Hz spectrogram hop rate.
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

      // Hot-path reads. Position is the ground truth for spectrogram
      // index — never subscribe via a React selector.
      const playback = usePlaybackStore.getState();
      const isPlaying = playback.status === "playing";
      const positionMs = playback.position * 1000;
      // Read mode every frame so cycling bars ↔ line takes effect on
      // the next paint without remounting.
      const mode = playback.visualizerMode;

      const scratch = scratchRef.current;
      if (isPlaying) {
        readFrameInto(frames, positionMs, scratch);
      } else {
        // Paused/stopped: decay toward zero so bars don't freeze at
        // the last value.
        scratch.fill(0);
      }

      // Spring-ease each bar toward its target height. Alpha is
      // computed once per frame, not per bar — Math.pow is not cheap
      // enough to run 256 times.
      const easeDt = Math.min(rawDelta, EASE_DT_CLAMP_MS);
      const dtRatio = easeDt / EASE_REFERENCE_DT_MS;
      const alphaAttack = easeDt > 0 ? 1 - Math.pow(1 - EASE_ATTACK, dtRatio) : 0;
      const alphaDecay = easeDt > 0 ? 1 - Math.pow(1 - EASE_DECAY, dtRatio) : 0;
      const current = currentRef.current;
      for (let i = 0; i < BAR_COUNT; i++) {
        const target = scratch[i] / 255;
        const prev = current[i];
        const alpha = target > prev ? alphaAttack : alphaDecay;
        current[i] = prev + (target - prev) * alpha;
      }

      // Accent colour from the cached ref; no per-frame style-recalc.
      const { r, g, b } = accentRef.current;

      ctx.globalAlpha = GLOBAL_ALPHA;

      // Gradient: opaque at the top edge where bars originate, fading
      // to transparent at their tips.
      const maxH = (h * BAR_MAX_HEIGHT_PCT) / 100;
      const grad = ctx.createLinearGradient(0, 0, 0, maxH);
      grad.addColorStop(0, `rgba(${r}, ${g}, ${b}, ${GRADIENT_TOP_OPACITY})`);
      grad.addColorStop(1, `rgba(${r}, ${g}, ${b}, ${GRADIENT_BOTTOM_OPACITY})`);
      ctx.fillStyle = grad;

      if (mode === "line") {
        // --- Line mode ---
        //
        // Moving-average the eased bar heights, then draw a filled
        // curve from the top edge down to the smoothed profile.
        // Quadratic midpoint segments (each point is a control, segment
        // midpoints are anchors) give a continuous curve without bezier
        // tension math. Mirror symmetry is preserved because `current`
        // is already mirrored.
        //
        // Noise decorrelation is skipped here: horizontal smoothing
        // averages the bass lockstep out on its own.
        const smoothed = smoothedLineRef.current;
        smoothLineInto(current, LINE_SMOOTHING_WINDOW, smoothed);

        const n = BAR_COUNT;
        const xStep = n > 1 ? w / (n - 1) : w;

        // Path: start top-left, thread a quadratic chain through the
        // smoothed heights, close via a mirror quad to the top-right,
        // then closePath across the top edge. The closing quads have
        // vertical tangents at (0,0) and (w,0) to match the implicit
        // top-edge closure without kinks.
        ctx.beginPath();
        ctx.moveTo(0, 0);
        // Left entry quad: tangent at (0,0) points straight down
        // toward control (0, bar[0]*maxH), matching the top edge's
        // implicit vertical closure.
        for (let i = 0; i < n - 1; i++) {
          const x1 = i * xStep;
          const y1 = smoothed[i] * maxH;
          const x2 = (i + 1) * xStep;
          const y2 = smoothed[i + 1] * maxH;
          ctx.quadraticCurveTo(x1, y1, (x1 + x2) / 2, (y1 + y2) / 2);
        }
        // Right exit quad: mirror of the entry. Control at
        // (w, bar[n-1]*maxH), anchor at (w, 0); vertical tangent at
        // (w, 0) matches the top edge.
        ctx.quadraticCurveTo(w, smoothed[n - 1] * maxH, w, 0);
        ctx.closePath();
        ctx.fill();
      } else {
        // --- Bars mode (default) ---
        //
        // Stroke is applied per-bar after the fill, only when width > 0.
        const drawBorder = BORDER_WIDTH_PX > 0 && BORDER_OPACITY > 0;
        if (drawBorder) {
          ctx.lineWidth = BORDER_WIDTH_PX;
          ctx.strokeStyle = `rgba(${r}, ${g}, ${b}, ${BORDER_OPACITY})`;
        }

        // Even spacing across full width; recomputed per frame for
        // resize.
        const totalGap = BAR_GAP_PX * (BAR_COUNT - 1);
        const barWidth = Math.max(0.5, (w - totalGap) / BAR_COUNT);

        // Bass decorrelation: sub-cutoff bands get a per-band
        // sinusoidal wobble so adjacent bars sharing an FFT bin don't
        // lockstep. Gated on amplitude so silent bars stay still
        // (near-zero bars would otherwise flicker at the bottom).
        // Applied per band (not per bar) so mirror halves stay in
        // sync.
        const noisePhases = noisePhasesRef.current;
        const noiseFreqs = noiseFreqsRef.current;
        const noiseActive = BASS_NOISE_AMOUNT > 0 && BASS_NOISE_BAND_CUTOFF > 0;
        const noiseT = noiseActive ? performance.now() / 1000 : 0;

        for (let i = 0; i < BAR_COUNT; i++) {
          let barH = current[i];

          if (noiseActive && noisePhases && noiseFreqs) {
            const band = barToBand(i);
            if (band < BASS_NOISE_BAND_CUTOFF && barH > BASS_NOISE_GATE) {
              const wobble = Math.sin(noiseT * noiseFreqs[band] + noisePhases[band]);
              barH = barH * (1 + wobble * BASS_NOISE_AMOUNT);
              if (barH < 0) barH = 0;
              else if (barH > 1) barH = 1;
            }
          }

          const barHeight = barH * maxH;
          if (barHeight < MIN_VISIBLE_HEIGHT_PX) continue;
          const x = i * (barWidth + BAR_GAP_PX);
          ctx.fillRect(x, 0, barWidth, barHeight);
          if (drawBorder) {
            ctx.strokeRect(x, 0, barWidth, barHeight);
          }
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
  }, [frames]);

  return (
    <div ref={containerRef} className="focus-visualizer">
      <canvas ref={canvasRef} />
    </div>
  );
}
