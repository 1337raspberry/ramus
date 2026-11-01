/**
 * Helpers for morphing the seek bar's waveform from one track's levels to
 * the next. Tracks carry different numbers of level samples, so both sides
 * are resampled to a shared count before blending point by point.
 */

/** Morph length. */
export const MORPH_MS = 450;

/** Most points a morph frame is drawn with; longer level arrays are
 * resampled down for the duration of the morph only. */
const MAX_MORPH_POINTS = 1024;

/** `levels` linearly resampled to `count` points (endpoints kept). An empty
 * input resamples to silence. */
export function resampleLevels(levels: readonly number[], count: number): number[] {
  if (count <= 0) return [];
  if (levels.length === 0) return new Array(count).fill(0);
  if (levels.length === 1 || count === 1) return new Array(count).fill(levels[0]);
  const out = new Array<number>(count);
  const scale = (levels.length - 1) / (count - 1);
  for (let i = 0; i < count; i++) {
    const pos = i * scale;
    const lo = Math.floor(pos);
    const hi = Math.min(lo + 1, levels.length - 1);
    const t = pos - lo;
    out[i] = levels[lo] * (1 - t) + levels[hi] * t;
  }
  return out;
}

/** The point count a morph between `a` and `b` is drawn with. */
export function morphPointCount(a: readonly number[], b: readonly number[]): number {
  return Math.min(Math.max(a.length, b.length, 2), MAX_MORPH_POINTS);
}

/** `a` blended towards `b` by `t` (0..1); both must be the same length. */
export function blendLevels(a: readonly number[], b: readonly number[], t: number): number[] {
  return a.map((v, i) => v + (b[i] - v) * t);
}

/** Smooth start and finish (cubic ease-in-out), for `t` in 0..1. */
export function easeInOut(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}
