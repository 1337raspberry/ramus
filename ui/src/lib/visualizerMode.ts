/**
 * Focus-mode visualiser modes.
 *
 * - `"ridge"` — a stack of spectrum-history lines rising from the bottom
 *   edge, bass left, treble right (the default)
 * - `"bars"`  — one bar per tap band from the top edge, mirrored: bass
 *   centred, treble at the edges
 * - `"off"`   — nothing drawn, and the spectrum tap removed
 *
 * The focus view's toggle steps through all three. The clear screen hides
 * everything but the visualiser, so it has no off: it draws the ridge in
 * off's place and its toggle steps between the two looks only. The
 * `disableSpectrum` setting overrides every mode.
 */
export type VisualizerMode = "ridge" | "bars" | "off";

export const DEFAULT_VISUALIZER_MODE: VisualizerMode = "ridge";

const CYCLE: readonly VisualizerMode[] = ["ridge", "bars", "off"];
const CLEAR_CYCLE: readonly VisualizerMode[] = ["ridge", "bars"];

/** The mode actually drawn for `mode`: itself, except the ridge for off in the clear screen. */
export function shownVisualizerMode(mode: VisualizerMode, clear: boolean): VisualizerMode {
  return clear && mode === "off" ? "ridge" : mode;
}

/**
 * The mode the toggle steps to from the one shown: ridge, bars, off and
 * round again, or ridge and bars alone in the clear screen.
 */
export function nextVisualizerMode(mode: VisualizerMode, clear: boolean): VisualizerMode {
  const cycle = clear ? CLEAR_CYCLE : CYCLE;
  const at = cycle.indexOf(shownVisualizerMode(mode, clear));
  return cycle[(at + 1) % cycle.length];
}

/** A persisted mode, or the default for anything unrecognised. */
export function parseVisualizerMode(raw: string | null): VisualizerMode {
  return raw === "ridge" || raw === "bars" || raw === "off" ? raw : DEFAULT_VISUALIZER_MODE;
}
