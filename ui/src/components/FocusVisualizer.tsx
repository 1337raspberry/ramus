import { useEffect, useRef } from "react";
import { usePlaybackStore } from "../stores/playbackStore";
import { useVisualizerDebugStore } from "../stores/visualizerDebugStore";

/**
 * Abstract hanging-line audio visualiser for the focus Now Playing view.
 *
 * Data source: libmpv's `af-metadata/astats` filter gives us left/right
 * RMS in dBFS via the `audio-level` Tauri event. The store mirrors this
 * into `audioLevels`, which this component reads via `getState()` inside
 * a requestAnimationFrame loop — deliberately bypassing React re-renders
 * so we get smooth ~60 fps animation without thrashing the component tree.
 *
 * All tunable shape / animation / appearance parameters live in
 * `visualizerDebugStore`. They are read via `getState()` each frame, so
 * changes from the live debug panel apply immediately. Permanent defaults
 * are baked into `VISUALIZER_DEFAULTS` in that store file — edit them
 * there (or use the debug panel's "Copy" action to generate a paste-ready
 * block) rather than hardcoding here.
 *
 * Rendering: a single smooth curve anchored at the top edge that drapes
 * downward. Each point's depth is driven by (real overall RMS) *
 * (drooping-V center bias) * (three-octave smooth noise), so the line as
 * a whole pulses with loudness while individual points wobble organically.
 * The area above the curve is filled with a fading gradient and the curve
 * itself is stroked for a crisp boundary.
 */

const MIN_DB = -60;
const MAX_DB = 0;

/** dBFS → 0..1 linear amplitude ratio (clamped). */
function dbToAmp(db: number): number {
  if (!Number.isFinite(db) || db <= MIN_DB) return 0;
  const clamped = Math.min(MAX_DB, db);
  return Math.min(1, Math.pow(10, clamped / 20));
}

/**
 * Smooth hash-based noise in [0, 1] — deterministic for a given (index,
 * phase) pair but continuous as `phase` advances. Cosine-interpolates
 * between two pseudo-random samples for C1 continuity.
 */
function smoothNoise(index: number, phase: number): number {
  const p = phase + index * 0.37;
  const i = Math.floor(p);
  const f = p - i;
  const hash = (n: number) => {
    const x = Math.sin(n * 127.1 + index * 311.7) * 43758.5453;
    return x - Math.floor(x);
  };
  const a = hash(i);
  const b = hash(i + 1);
  const t = (1 - Math.cos(f * Math.PI)) * 0.5;
  return a * (1 - t) + b * t;
}

export default function FocusVisualizer() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number>(0);
  // Current normalised depth (0 = touching the top edge, 1 = bottom of canvas)
  // for each sample point, spring-eased toward the target each frame. Sized
  // to the current pointCount; re-allocated if the debug panel changes it.
  const currentRef = useRef<Float32Array>(new Float32Array(64));

  useEffect(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // Track DPR alongside dimensions so the canvas reconfigures when the
    // window moves between displays of different pixel densities (1x ↔ 2x
    // Retina). Reading dpr fresh on every resize means the backing store
    // always matches the current display.
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

    const render = (now: number) => {
      const { w, h } = resize();
      ctx.clearRect(0, 0, w, h);

      // Hot-path reads — no React subscriptions
      const playback = usePlaybackStore.getState();
      // DEBUG (focus visualiser panel): live-tunable params. When the debug
      // panel is removed, change this to:
      //     const params = VISUALIZER_DEFAULTS;
      // and import VISUALIZER_DEFAULTS directly from visualizerDebugStore.
      const params = useVisualizerDebugStore.getState();

      // Resize the eased-depth buffer if the point count slider changed
      const pointCount = Math.max(2, Math.floor(params.pointCount));
      if (currentRef.current.length !== pointCount) {
        currentRef.current = new Float32Array(pointCount);
      }

      const levels = playback.audioLevels;
      const isPlaying = playback.status === "playing";

      // Overall intensity from real astats metering
      let mid = 0;
      if (levels && isPlaying) {
        const lr = dbToAmp(levels.leftRms);
        const rr = dbToAmp(levels.rightRms);
        mid = (lr + rr) / 2;
      }

      // Calm idle breathing when silent / paused
      const idleDepth = params.idleBase + (Math.sin(now / 1400) * 0.5 + 0.5) * params.idleAmplitude;

      const phaseA = now * params.phaseRateA;
      const phaseB = now * params.phaseRateB;
      const phaseC = now * params.phaseRateC;

      const current = currentRef.current;
      const halfPoints = (pointCount - 1) / 2;

      for (let i = 0; i < pointCount; i++) {
        const a = smoothNoise(i * params.spatialA, phaseA);
        const b = smoothNoise(i * params.spatialB + 1000, phaseB);
        const c = smoothNoise(i * params.spatialC + 2000, phaseC);

        let shape = a * params.weightA + b * params.weightB + c * params.weightC;
        shape = Math.pow(Math.max(0, shape), params.shapePower);

        const distFromCenter = halfPoints > 0 ? Math.abs(i - halfPoints) / halfPoints : 0;
        const centerBias = 1 - Math.pow(distFromCenter, params.centerPower) * params.centerStrength;

        let target: number;
        if (mid > 0.01) {
          target =
            mid *
              params.musicDepth *
              centerBias *
              (params.shapeMixBase + shape * params.shapeMixRange) +
            idleDepth * params.idleWhileMusic;
        } else {
          target = idleDepth * shape * centerBias;
        }

        if (target > 1) target = 1;
        if (target < 0) target = 0;

        const prev = current[i];
        const ease = target > prev ? params.attackEase : params.decayEase;
        current[i] = prev + (target - prev) * ease;
      }

      // Convert normalised depths to canvas coordinates. The canvas spans
      // the entire focus window (the visualiser is a background layer), so
      // we scale each point's normalised 0..1 depth by `maxDepthPct` of
      // the full canvas height — the curve drapes only within the top
      // `maxDepthPct` percent of the window, regardless of canvas size.
      const maxDrape = (h * params.maxDepthPct) / 100;
      const points: { x: number; y: number }[] = [];
      for (let i = 0; i < pointCount; i++) {
        const x = pointCount > 1 ? (i / (pointCount - 1)) * w : w / 2;
        const y = current[i] * maxDrape;
        points.push({ x, y });
      }

      // Accent colour from CSS variables
      const styles = getComputedStyle(document.documentElement);
      const r = styles.getPropertyValue("--accent-r").trim() || "120";
      const g = styles.getPropertyValue("--accent-g").trim() || "90";
      const b = styles.getPropertyValue("--accent-b").trim() || "220";

      // --- Fill region: top edge → curve → top edge
      ctx.beginPath();
      ctx.moveTo(0, 0);
      ctx.lineTo(points[0].x, points[0].y);
      for (let i = 0; i < pointCount - 1; i++) {
        const midX = (points[i].x + points[i + 1].x) / 2;
        const midY = (points[i].y + points[i + 1].y) / 2;
        ctx.quadraticCurveTo(points[i].x, points[i].y, midX, midY);
      }
      ctx.lineTo(points[pointCount - 1].x, points[pointCount - 1].y);
      ctx.lineTo(w, 0);
      ctx.closePath();

      // Gradient fades over the drape region specifically, not the full
      // canvas — so the fill looks cohesive regardless of how tall the
      // underlying canvas is.
      const gradEnd = Math.max(1, maxDrape);
      const grad = ctx.createLinearGradient(0, 0, 0, gradEnd);
      grad.addColorStop(0, `rgba(${r}, ${g}, ${b}, ${params.fillOpacity})`);
      grad.addColorStop(1, `rgba(${r}, ${g}, ${b}, 0)`);
      ctx.fillStyle = grad;
      ctx.fill();

      // --- Stroke the curve itself
      ctx.beginPath();
      ctx.moveTo(points[0].x, points[0].y);
      for (let i = 0; i < pointCount - 1; i++) {
        const midX = (points[i].x + points[i + 1].x) / 2;
        const midY = (points[i].y + points[i + 1].y) / 2;
        ctx.quadraticCurveTo(points[i].x, points[i].y, midX, midY);
      }
      ctx.lineTo(points[pointCount - 1].x, points[pointCount - 1].y);

      ctx.strokeStyle = `rgba(${r}, ${g}, ${b}, ${params.strokeOpacity})`;
      ctx.lineWidth = params.strokeWidth;
      ctx.lineCap = "round";
      ctx.lineJoin = "round";
      ctx.stroke();

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
    </div>
  );
}
