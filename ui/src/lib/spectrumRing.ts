import type { SpectrumFramesPayload } from "./types";

/**
 * Position-keyed ring of live spectrum frames from the `--af` tap.
 *
 * Kept outside Zustand on purpose: frames land ~60 times a second and the
 * only reader is the focus visualiser's requestAnimationFrame loop, which
 * pulls straight from here. Routing them through `set()` would wake every
 * store subscriber per frame for nothing.
 *
 * Frames arrive in bursts, up to mpv's `audio-buffer` (0.5 s) ahead of the
 * reported playback position. The ring holds ~1.6 s at 60 fps so the
 * lookup always has the frame for "now" with margin on both sides, and a
 * seek's stale frames simply age out.
 */
export interface SpectrumRingFrame {
  /** Track-timeline position of the frame's first sample, in seconds. */
  pos: number;
  /**
   * One quantised bar height (0..255) per band per channel: the left
   * channel's bands followed by the right channel's.
   */
  bands: Uint8Array;
}

const RING_CAPACITY = 96;

const ring: (SpectrumRingFrame | null)[] = new Array<SpectrumRingFrame | null>(RING_CAPACITY).fill(
  null,
);
let head = 0;
let count = 0;
let bandCount = 0;
let lastPushAt = 0;

/** Append a burst of frames (oldest first) from the `spectrum-frames` event. */
export function pushSpectrumFrames(payload: SpectrumFramesPayload): void {
  bandCount = payload.bandCount;
  for (const f of payload.frames) {
    ring[head] = {
      pos: f.pos,
      bands: f.bands instanceof Uint8Array ? f.bands : Uint8Array.from(f.bands),
    };
    head = (head + 1) % RING_CAPACITY;
    if (count < RING_CAPACITY) count += 1;
  }
  lastPushAt = performance.now();
}

/** Drop every frame (track change, queue cleared). */
export function clearSpectrumRing(): void {
  ring.fill(null);
  head = 0;
  count = 0;
}

/** Bands per channel of the most recent burst; 0 until the first one lands. */
export function spectrumBandCount(): number {
  return bandCount;
}

/** `performance.now()` of the most recent push; 0 if none yet. */
export function spectrumLastPushAt(): number {
  return lastPushAt;
}

/**
 * The frame closest to `target` (seconds) within the window
 * `[target - lagSec, target + leadSec]`, or null when nothing is that
 * close. Linear scan: the ring is small and this runs once per paint.
 */
export function pickSpectrumFrame(
  target: number,
  lagSec: number,
  leadSec: number,
): SpectrumRingFrame | null {
  let best: SpectrumRingFrame | null = null;
  let bestDist = Infinity;
  for (let i = 0; i < count; i++) {
    const f = ring[i];
    if (!f) continue;
    const d = f.pos - target;
    if (d < -lagSec || d > leadSec) continue;
    const dist = Math.abs(d);
    if (dist < bestDist) {
      bestDist = dist;
      best = f;
    }
  }
  return best;
}
