import { useSyncExternalStore } from "react";
import { VISUALIZER_PARAMS, type VisualizerParams } from "../lib/visualizerParams";

/**
 * Live tuning state behind the development-only visualiser panel.
 *
 * Bar values are written straight into `VISUALIZER_PARAMS`, the object the
 * paint loop reads, so a slider drag changes the next frame without a
 * React render. Art values have no production-side object: `FocusVizDebug`
 * applies them through an injected stylesheet, and the tap value is pushed
 * to the backend over a command. All persist to `localStorage` so a tuning
 * pass survives reloads. This module is only ever loaded in development
 * builds.
 */

/** Art layout values; production reads the equivalents from styles.css. */
export interface ArtTuning {
  /** Art edge as a fraction of the largest square that fits its panel. */
  artScale: number;
  /** Art opacity, 0..1. */
  artOpacity: number;
  /** Vertical nudge as a fraction of the art's own height; positive is down. */
  artOffsetY: number;
  /** Width of the art column relative to the controls column, in `fr`. */
  artColumn: number;
}

/** Backend tap values; production reads the constants in `spectrum_tap.rs`. */
export interface TapTuning {
  /** Spectral tilt in dB per octave (`TILT_DB_PER_OCTAVE`). */
  tapTilt: number;
}

export type FocusVizTuning = VisualizerParams & ArtTuning & TapTuning;

/**
 * The shipped art layout, mirrored from styles.css: `--art-scale` on
 * `.focus-art-container`, the container's own opacity and transform, and
 * the `.focus-body` grid columns.
 */
const ART_DEFAULTS: Readonly<ArtTuning> = Object.freeze({
  artScale: 0.93,
  artOpacity: 1,
  artOffsetY: 0,
  artColumn: 1,
});

/** The shipped tap values, mirrored from `spectrum_tap.rs`. */
const TAP_DEFAULTS: Readonly<TapTuning> = Object.freeze({
  tapTilt: 1.0,
});

/** Shipped values, captured before anything here touches the params. */
export const TUNING_DEFAULTS: Readonly<FocusVizTuning> = Object.freeze({
  ...VISUALIZER_PARAMS,
  ...ART_DEFAULTS,
  ...TAP_DEFAULTS,
});

const STORAGE_KEY = "ramus.dev.focusVizTuning";

const art: ArtTuning = { ...ART_DEFAULTS };
const tap: TapTuning = { ...TAP_DEFAULTS };

function current(): FocusVizTuning {
  return { ...VISUALIZER_PARAMS, ...art, ...tap };
}

/** Immutable copy replaced on every change, for `useSyncExternalStore`. */
let snapshot: FocusVizTuning = current();
const listeners = new Set<() => void>();

function isArtKey(key: keyof FocusVizTuning): key is keyof ArtTuning {
  return key in ART_DEFAULTS;
}

function isTapKey(key: keyof FocusVizTuning): key is keyof TapTuning {
  return key in TAP_DEFAULTS;
}

function write(key: keyof FocusVizTuning, value: number): void {
  if (isArtKey(key)) {
    art[key] = value;
  } else if (isTapKey(key)) {
    tap[key] = value;
  } else {
    VISUALIZER_PARAMS[key] = value;
  }
}

function notify(): void {
  snapshot = current();
  for (const fn of listeners) fn();
}

function persist(): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(snapshot));
  } catch {
    // Storage unavailable; the values still apply for this session.
  }
}

/** Set one value. Non-finite numbers are ignored. */
export function setTuningValue(key: keyof FocusVizTuning, value: number): void {
  if (!Number.isFinite(value) || snapshot[key] === value) return;
  write(key, value);
  notify();
  persist();
}

/** Set several values as one change. Non-finite numbers are ignored. */
export function setTuningValues(values: Partial<FocusVizTuning>): void {
  let changed = false;
  for (const [key, value] of Object.entries(values) as [keyof FocusVizTuning, number][]) {
    if (!Number.isFinite(value) || snapshot[key] === value) continue;
    write(key, value);
    changed = true;
  }
  if (!changed) return;
  notify();
  persist();
}

/** Restore the shipped values. */
export function resetTuning(): void {
  for (const key of Object.keys(TUNING_DEFAULTS) as (keyof FocusVizTuning)[]) {
    write(key, TUNING_DEFAULTS[key]);
  }
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Nothing to clear.
  }
  notify();
}

/** The current values, for copying out. */
export function tuningSnapshot(): FocusVizTuning {
  return snapshot;
}

function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

/** React view of the current values; re-renders the caller on change. */
export function useTuning(): FocusVizTuning {
  return useSyncExternalStore(subscribe, () => snapshot);
}

// Restore the previous session's values. Only known keys holding finite
// numbers are taken.
try {
  const raw = localStorage.getItem(STORAGE_KEY);
  if (raw) {
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    for (const key of Object.keys(TUNING_DEFAULTS) as (keyof FocusVizTuning)[]) {
      const v = parsed[key];
      if (typeof v === "number" && Number.isFinite(v)) write(key, v);
    }
    snapshot = current();
  }
} catch {
  // Corrupt or unavailable storage: keep the defaults.
}
