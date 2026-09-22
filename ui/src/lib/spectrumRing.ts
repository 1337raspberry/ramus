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
 *
 * Every frame carries the stream epoch it was cut from, and the lookup is
 * clocked by the `playback-audible` tick (mpv's `audio-pts`) rather than
 * the seek-bar position: across a gapless join mpv moves to the next
 * track about an audio buffer before the join is heard, and the seek-bar
 * position sits at 0 for that window while the audible position runs
 * negative on the new track's timeline. Mapping a negative audible
 * position back onto the end of the previous epoch keeps the bars on the
 * outgoing track until the join is actually heard.
 */
export interface SpectrumRingFrame {
  /** Stream epoch the frame belongs to (see `pushSpectrumFrames`). */
  epoch: number;
  /** Track-timeline position of the frame's first sample, in seconds. */
  pos: number;
  /**
   * One quantised bar height (0..255) per band per channel: the left
   * channel's bands followed by the right channel's.
   */
  bands: Uint8Array;
}

const RING_CAPACITY = 96;

// Slots are allocated once and overwritten in place: a burst lands every
// few frames, and a fresh object plus typed array per frame would just be
// garbage for the collector.
const ring: SpectrumRingFrame[] = Array.from({ length: RING_CAPACITY }, () => ({
  epoch: 0,
  pos: 0,
  bands: new Uint8Array(0),
}));
// Slots `0..count` are the filled ones: `head` starts at 0 and a clear
// resets it to 0, so the filled region is always a prefix of the array.
let head = 0;
let count = 0;
let lastPushAt = 0;

// Position of the latest frame seen per epoch, for the current epoch and
// the one before it: the end of the previous stream is where a negative
// audible position on the current one maps to.
const epochEnd = new Map<number, number>();

interface AudibleClock {
  epoch: number;
  position: number;
  /** `performance.now()` when the tick arrived. */
  at: number;
}
let clock: AudibleClock | null = null;

/** Append a burst of frames (oldest first) from the `spectrum-frames` event. */
export function pushSpectrumFrames(payload: SpectrumFramesPayload): void {
  // The backend states the shape; a frame that disagrees with it (a band
  // count change mid-flight) is dropped rather than drawn as noise.
  const width = payload.bandCount * payload.channels;
  for (const f of payload.frames) {
    if (f.bands.length !== width) continue;
    const slot = ring[head];
    slot.epoch = payload.epoch;
    slot.pos = f.pos;
    if (slot.bands.length !== width) slot.bands = new Uint8Array(width);
    slot.bands.set(f.bands);
    head = (head + 1) % RING_CAPACITY;
    if (count < RING_CAPACITY) count += 1;
    const end = epochEnd.get(payload.epoch);
    if (end === undefined || f.pos > end) epochEnd.set(payload.epoch, f.pos);
  }
  for (const epoch of epochEnd.keys()) {
    if (epoch < payload.epoch - 1) epochEnd.delete(epoch);
  }
  lastPushAt = performance.now();
}

/** Drop every frame and the audible clock (playback stopped, queue cleared). */
export function clearSpectrumRing(): void {
  head = 0;
  count = 0;
  epochEnd.clear();
  clock = null;
}

/** Record a `playback-audible` tick: the position being heard and its epoch. */
export function setAudibleClock(epoch: number, position: number): void {
  clock = { epoch, position, at: performance.now() };
}

/**
 * Where the audio is right now, extrapolated from the last audible tick:
 * the epoch to look in and the position within it. A negative position on
 * the current epoch is still the previous stream's tail and is mapped
 * onto the end of that stream. Null until the first tick arrives.
 */
export function audibleTarget(now: number): { epoch: number; pos: number } | null {
  if (!clock) return null;
  let epoch = clock.epoch;
  let pos = clock.position + (now - clock.at) / 1000;
  if (pos < 0) {
    const end = epochEnd.get(epoch - 1);
    if (end !== undefined) {
      epoch -= 1;
      pos += end;
    }
  }
  return { epoch, pos };
}

/** `performance.now()` of the most recent push; 0 if none yet. */
export function spectrumLastPushAt(): number {
  return lastPushAt;
}

/**
 * The frame of `epoch` (any epoch when null) closest to `target` (seconds)
 * within the window `[target - lagSec, target + leadSec]`, or null when
 * nothing is that close. Linear scan: the ring is small and this runs
 * once per paint. The result is a ring slot the next burst may overwrite,
 * so read it within the same task.
 */
export function pickSpectrumFrame(
  epoch: number | null,
  target: number,
  lagSec: number,
  leadSec: number,
): SpectrumRingFrame | null {
  let best: SpectrumRingFrame | null = null;
  let bestDist = Infinity;
  for (let i = 0; i < count; i++) {
    const f = ring[i];
    if (epoch !== null && f.epoch !== epoch) continue;
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
