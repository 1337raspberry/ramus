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
 * reported playback position. The ring holds ~2 s at 60 fps so the lookup
 * always has the frame for "now" with margin on both sides, even across a
 * gapless join where the incoming stream's frames share the ring with the
 * outgoing stream's still-audible tail, and a seek's stale frames simply
 * age out.
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

const RING_CAPACITY = 128;

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

/**
 * A clock tick older than this is no clock at all. Ticks land about
 * twenty times a second while audio flows (mpv's playloop cadence), so a
 * gap this long means the stream behind the clock has stopped: a skip
 * has replaced it, or playback paused. Past it the lookup falls back to
 * the seek-bar position rather than running the dead stream's timeline
 * on, which would otherwise draw the frames the tap cut ahead of the
 * last audible moment (up to an audio buffer's worth) for as long as the
 * next stream takes to tick.
 */
const AUDIBLE_CLOCK_STALE_MS = 250;

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
 * Where the audio will be `aheadSec` after `now`, extrapolated from the
 * last audible tick: the epoch to look in and the position within it. A
 * negative position on the current epoch is still the previous stream's
 * tail and is mapped onto the end of that stream. Null until the first
 * tick arrives, and again once the last tick is stale at `now`
 * (`AUDIBLE_CLOCK_STALE_MS`).
 */
export function audibleTarget(now: number, aheadSec = 0): { epoch: number; pos: number } | null {
  if (!clock || now - clock.at > AUDIBLE_CLOCK_STALE_MS) return null;
  let epoch = clock.epoch;
  let pos = clock.position + (now - clock.at) / 1000 + aheadSec;
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
function pickSpectrumFrame(
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

// The frame at each step past the base frame, and the bands put together
// from them; both reused by every read so a paint allocates nothing.
const stepFrames: SpectrumRingFrame[] = [];
let composite = new Uint8Array(0);

/**
 * The bands to draw for `target`: the frame `pickSpectrumFrame` finds,
 * with each band read instead from the frame `ahead[k]` frames after it
 * in the same stream (`periodSec` apart). A narrow bass band's level
 * swells for tens of milliseconds after the note it measures starts, so
 * reading it that much further on lands its onsets with the treble's.
 * `ahead` holds one step count per band and applies on every channel; a
 * later frame the ring doesn't hold (the stream ended, or a seek) falls
 * back to the latest one before it that it does. Null when there is no
 * frame near `target`. The result is scratch the next read overwrites
 * (or, with nothing to step, a ring slot the next burst may), so read it
 * within the same task.
 */
export function readSpectrumBands(
  epoch: number | null,
  target: number,
  lagSec: number,
  leadSec: number,
  ahead: Uint8Array | null,
  periodSec: number,
): Uint8Array | null {
  const base = pickSpectrumFrame(epoch, target, lagSec, leadSec);
  if (!base) return null;
  const width = base.bands.length;
  const n = ahead ? ahead.length : 0;
  if (!ahead || n === 0 || width % n !== 0 || !(periodSec > 0)) return base.bands;

  let maxStep = 0;
  for (let k = 0; k < n; k++) if (ahead[k] > maxStep) maxStep = ahead[k];
  if (maxStep === 0) return base.bands;
  stepFrames.length = maxStep + 1;
  stepFrames[0] = base;
  for (let m = 1; m <= maxStep; m++) {
    const want = base.pos + m * periodSec;
    let found = stepFrames[m - 1];
    for (let i = 0; i < count; i++) {
      const f = ring[i];
      if (
        f.epoch === base.epoch &&
        f.bands.length === width &&
        Math.abs(f.pos - want) < periodSec / 2
      ) {
        found = f;
        break;
      }
    }
    stepFrames[m] = found;
  }

  if (composite.length !== width) composite = new Uint8Array(width);
  for (let c = 0; c < width; c += n) {
    for (let k = 0; k < n; k++) composite[c + k] = stepFrames[ahead[k]].bands[c + k];
  }
  return composite;
}
