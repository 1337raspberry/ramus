import { playHaptic } from "./commands";

/**
 * The haptic effects the native side knows:
 *
 * - `pop`: a transport tap.
 * - `thud`: the now-playing sheet landing open or shut.
 * - `bump`: the seek bar's thumb reaching either end.
 * - `grain`: one tick of travel along the seek bar.
 * - `holdStart`: a rumble that builds over `value` milliseconds, until
 *   `holdFire` (the long press fired) or `holdCancel`. Sent through
 *   `beginHoldRumble`.
 */
export type HapticEffect =
  | "pop"
  | "thud"
  | "bump"
  | "grain"
  | "holdStart"
  | "holdCancel"
  | "holdFire";

/** Only the iPhone build has a native side that plays these; elsewhere a
 * call would be a wasted round trip to the backend (a scrub sends ~30 a
 * second). */
const HAS_HAPTICS = /iPhone|iPad|iPod/.test(navigator.userAgent);

/** The hold effects in flight. Each command runs on its own backend task, so
 * a quick tap's `holdStart` and `holdCancel` could otherwise land in either
 * order, and a cancel that arrives first leaves the rumble playing. */
let holdRequests: Promise<unknown> = Promise.resolve();

/** Play `effect` without waiting for it. Does nothing where the platform has
 * no haptics, and never throws: a gesture must not depend on it. */
export function haptic(effect: HapticEffect, value?: number): void {
  if (!HAS_HAPTICS) return;
  if (effect === "holdStart" || effect === "holdCancel" || effect === "holdFire") {
    const send = () => playHaptic(effect, value);
    holdRequests = holdRequests.then(send, send).catch(() => {});
    return;
  }
  playHaptic(effect, value).catch(() => {});
}

/** How long a hold lasts before its rumble begins. A touch that turns into a
 * scroll is over inside this, so scrolling a list of holdable rows stays
 * silent. */
export const HOLD_RUMBLE_DELAY_MS = 150;

export interface HoldRumble {
  /** The hold was given up: stop the rumble, or never start it. */
  cancel(): void;
  /** The hold fired: end the rumble on a pop. */
  fire(): void;
}

/**
 * The haptics under a pending long press of `holdMs`: nothing for `delayMs`,
 * then a rumble that builds until the hold fires. Call `cancel` or `fire`
 * exactly once.
 */
export function beginHoldRumble(holdMs: number, delayMs = HOLD_RUMBLE_DELAY_MS): HoldRumble {
  let started = false;
  let timer: number | null = window.setTimeout(() => {
    timer = null;
    started = true;
    haptic("holdStart", holdMs - delayMs);
  }, delayMs);
  const disarm = () => {
    if (timer !== null) {
      window.clearTimeout(timer);
      timer = null;
    }
  };
  return {
    cancel() {
      disarm();
      if (started) haptic("holdCancel");
      started = false;
    },
    fire() {
      disarm();
      started = false;
      haptic("holdFire");
    },
  };
}
