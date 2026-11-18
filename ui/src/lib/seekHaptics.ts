/** What one touch move along the seek bar should feel like. */
export type SeekTick = "grain" | "bump" | null;

export interface SeekHaptics {
  /** Touch-down at `xPx`, with the bar fraction (0..1) under it. Never ticks. */
  begin(xPx: number, frac: number, nowMs: number): void;
  /** A touch move; `frac` is already clamped to 0..1. */
  move(xPx: number, frac: number, nowMs: number): SeekTick;
}

/**
 * Turns thumb travel along the seek bar into haptic ticks: a grain tick each
 * `stepPx` of travel, at most one per `minIntervalMs`, and a single bump on
 * arriving at either end of the bar. The bump is spent until the thumb leaves
 * that end, so a finger resting or wandering past the end stays quiet.
 */
export function createSeekHaptics(
  opts: { stepPx?: number; minIntervalMs?: number } = {},
): SeekHaptics {
  const stepPx = opts.stepPx ?? 8;
  const minIntervalMs = opts.minIntervalMs ?? 33;
  let lastX = 0;
  let lastMs = 0;
  /** The end of the bar the thumb is resting on, once its bump has played. */
  let restingEnd: 0 | 1 | null = null;

  const endAt = (frac: number): 0 | 1 | null => (frac <= 0 ? 0 : frac >= 1 ? 1 : null);

  return {
    begin(xPx, frac, nowMs) {
      lastX = xPx;
      lastMs = nowMs;
      restingEnd = endAt(frac);
    },
    move(xPx, frac, nowMs) {
      const end = endAt(frac);
      if (end !== null) {
        if (end === restingEnd) return null;
        restingEnd = end;
        lastX = xPx;
        lastMs = nowMs;
        return "bump";
      }
      restingEnd = null;
      if (Math.abs(xPx - lastX) < stepPx || nowMs - lastMs < minIntervalMs) return null;
      lastX = xPx;
      lastMs = nowMs;
      return "grain";
    },
  };
}
