import { useSyncExternalStore } from "react";
import { VISUALIZER_PARAMS, type VisualizerParams } from "../lib/visualizerParams";
import { SINGALONG_PARAMS } from "../lib/singalong";

/**
 * Live tuning state behind the development-only visualiser panel.
 *
 * Bar values are written straight into `VISUALIZER_PARAMS`, the object the
 * paint loop reads, so a slider drag changes the next frame without a
 * React render. Art values have no production-side object: `FocusVizDebug`
 * applies them through an injected stylesheet, and the tap value is pushed
 * to the backend over a command. Lyrics-strip values are held here and
 * applied by `FocusVizDebug`, partly to `SINGALONG_PARAMS` and partly
 * through an injected stylesheet. All persist to `localStorage` so a
 * tuning pass survives reloads. This module is only ever loaded in
 * development builds.
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

/**
 * Clear-screen lyrics strip values. The first five override styles.css
 * (`.focus-singalong` and its lines); the offsets are margins, so they
 * add to the strip's position in every layout (windowed, full screen,
 * narrow). The rest mirror `SINGALONG_PARAMS`.
 */
export interface LyricsTuning {
  /** Preferred type size of the focus line, in vw (the clamp's middle term). */
  lyricsSizeVw: number;
  /** Smallest the focus line's type gets, in px. */
  lyricsMinPx: number;
  /** Largest the focus line's type gets, in px. */
  lyricsMaxPx: number;
  /** Moves the strip down (positive) or up, in px. */
  lyricsTopPx: number;
  /** Moves the strip's left edge in (positive) or out, in px. */
  lyricsLeftPx: number;
  /** Moves the strip's right edge in (positive) or out, in px. */
  lyricsRightPx: number;
  lyricsNeighbourScale: number;
  lyricsNeighbourOpacity: number;
  lyricsGapPx: number;
  lyricsLeadS: number;
}

export type FocusVizTuning = VisualizerParams & ArtTuning & TapTuning & LyricsTuning;

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

/**
 * The shipped lyrics-strip values: the type clamp mirrored from
 * `.focus-singalong-line` in styles.css, and `SINGALONG_PARAMS` captured
 * before `FocusVizDebug` writes to it.
 */
const LYRICS_DEFAULTS: Readonly<LyricsTuning> = Object.freeze({
  lyricsSizeVw: 3.3,
  lyricsMinPx: 34,
  lyricsMaxPx: 68,
  lyricsTopPx: 0,
  lyricsLeftPx: 0,
  lyricsRightPx: 0,
  lyricsNeighbourScale: SINGALONG_PARAMS.neighbourScale,
  lyricsNeighbourOpacity: SINGALONG_PARAMS.neighbourOpacity,
  lyricsGapPx: SINGALONG_PARAMS.gapPx,
  lyricsLeadS: SINGALONG_PARAMS.leadS,
});

/** Shipped values, captured before anything here touches the params. */
export const TUNING_DEFAULTS: Readonly<FocusVizTuning> = Object.freeze({
  ...VISUALIZER_PARAMS,
  ...ART_DEFAULTS,
  ...TAP_DEFAULTS,
  ...LYRICS_DEFAULTS,
});

const STORAGE_KEY = "ramus.dev.focusVizTuning";

const art: ArtTuning = { ...ART_DEFAULTS };
const tap: TapTuning = { ...TAP_DEFAULTS };
const lyrics: LyricsTuning = { ...LYRICS_DEFAULTS };

function current(): FocusVizTuning {
  return { ...VISUALIZER_PARAMS, ...art, ...tap, ...lyrics };
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

function isLyricsKey(key: keyof FocusVizTuning): key is keyof LyricsTuning {
  return key in LYRICS_DEFAULTS;
}

function write(key: keyof FocusVizTuning, value: number): void {
  if (isArtKey(key)) {
    art[key] = value;
  } else if (isTapKey(key)) {
    tap[key] = value;
  } else if (isLyricsKey(key)) {
    lyrics[key] = value;
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
