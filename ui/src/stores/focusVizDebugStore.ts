// DEBUG (focus viz tuning) — entire file is debug scaffolding.
//
// In-memory + localStorage-persisted store backing the slider panel
// for live-tuning the focus-mode FFT visualiser. The whole thing is
// throwaway: when you've found values you like and want to bake them
// in, follow this checklist:
//
//   1. Note the values you've settled on (e.g. via the panel's
//      "Current values" readout — or just check localStorage key
//      `focus-viz-debug` in DevTools).
//   2. Delete this file.
//   3. Delete `ui/src/components/FocusVisualizerDebugPanel.tsx`.
//   4. Delete the `.focus-viz-debug*` CSS block in `styles.css`
//      (search for `DEBUG (focus viz tuning)` to find it).
//   5. In `ui/src/components/FocusVisualizer.tsx`, search for
//      `DEBUG (focus viz tuning)` and replace each store read with
//      the constant value you settled on. The original constants
//      block at the top of the file gives you the names to use.
//   6. In `ui/src/components/FocusNowPlayingView.tsx`, search for
//      `DEBUG (focus viz tuning)` and remove the panel mount + the
//      ⌘⇧V keyboard handler.
//   7. Run `cd ui && npm run build` to confirm nothing references
//      the deleted store/panel/CSS.
//
// One grep over the codebase for `DEBUG (focus viz tuning)` should
// surface every removal site.

import { create } from "zustand";
import { persist } from "zustand/middleware";

export interface FocusVizDebugState {
  // --- Layout ---
  /** Max bar height as % of the focus-window viewport. */
  barMaxHeightPct: number;
  /** Horizontal gap between adjacent bars in CSS px. */
  barGapPx: number;
  /** Skip drawing bars below this height (cleans up flicker). */
  minVisibleHeightPx: number;
  /** Vertical position of the "Analysing audio…" placeholder, in vh. */
  placeholderTopVh: number;

  // --- Animation ---
  /** Spring ease for rising bars (transient hits). 0..1, higher = snappier. */
  easeAttack: number;
  /** Spring ease for falling bars (decay tails). 0..1, higher = snappier. */
  easeDecay: number;

  // --- Colour / fill ---
  /** Gradient stop 0 alpha (top of bars, where they originate). */
  gradientTopOpacity: number;
  /** Gradient stop 1 alpha (bottom tips of bars, where they fade out). */
  gradientBottomOpacity: number;
  /** Multiplier applied to the whole canvas via ctx.globalAlpha. */
  globalAlpha: number;
  /** Stroke width around each bar in CSS px. 0 = no border. */
  borderWidthPx: number;
  /** Border alpha (only used if borderWidthPx > 0). */
  borderOpacity: number;

  // --- Bass decorrelation ---
  // The first ~47 log-spaced bands all share a single FFT bin at our
  // default settings (21.5 Hz bin width > 2-3 Hz band width), so they
  // return identical values and move in lockstep visually — 10+ bars in
  // the centre of the mirrored display pulsing as one block. Fix is a
  // tiny per-band sinusoidal wobble, applied after easing, gated on
  // amplitude so silent bars don't twitch. See the big comment near the
  // draw loop in FocusVisualizer.tsx for the physics rationale.
  /** Peak amplitude of the wobble, as a fraction of the bar's height. */
  bassNoiseAmount: number;
  /** Noise applies to bands with index ≤ this value. 0 = disabled. */
  bassNoiseBandCutoff: number;
  /** Minimum bar height (0..1) required before noise kicks in. */
  bassNoiseGate: number;

  // --- Panel state (not persisted — see merge() below) ---
  /** Whether the slider panel is visible. Toggled with ⌘⇧V. */
  panelOpen: boolean;
}

export const FOCUS_VIZ_DEFAULTS: FocusVizDebugState = {
  // Layout — baked values: keep the viz subtle so it decorates rather
  // than dominates the focus view.
  barMaxHeightPct: 20,
  barGapPx: 1,
  minVisibleHeightPx: 0.5,
  placeholderTopVh: 1.5,
  // Animation
  easeAttack: 0.55,
  easeDecay: 0.35,
  // Colour — baked values: mid/low alpha so the viz reads as a ghost
  // silhouette rather than a foreground element.
  gradientTopOpacity: 0.95,
  gradientBottomOpacity: 0.3,
  globalAlpha: 0.3,
  borderWidthPx: 0,
  borderOpacity: 0,
  // Bass decorrelation — baked values. 8% sinusoidal wobble applied
  // to the first 40 bands when they're above 8% of max height, so
  // silent bars stay still and active bass bars drift out of the
  // lockstep caused by the first ~47 log-spaced bands sharing a
  // single FFT bin at our default settings.
  bassNoiseAmount: 0.08,
  bassNoiseBandCutoff: 40,
  bassNoiseGate: 0.08,
  // Panel
  panelOpen: false,
};

export const useFocusVizDebugStore = create<FocusVizDebugState>()(
  persist(() => ({ ...FOCUS_VIZ_DEFAULTS }), {
    name: "focus-viz-debug",
    // Bumping this when the baked defaults change forces zustand to
    // discard any older persisted state and rehydrate from the new
    // defaults. Saves the user from having to hit Reset manually after
    // a default change. Increment on every baked-defaults update.
    version: 3,
    // Always start with the panel closed regardless of last session.
    merge: (persisted, current) => ({
      ...current,
      ...(persisted as Partial<FocusVizDebugState>),
      panelOpen: false,
    }),
  }),
);

/** Imperative setter — used by the slider panel. */
export function setFocusVizParam<K extends keyof FocusVizDebugState>(
  key: K,
  value: FocusVizDebugState[K],
): void {
  useFocusVizDebugStore.setState({ [key]: value } as Partial<FocusVizDebugState>);
}

/** Reset every tunable param back to its default. */
export function resetFocusVizDebug(): void {
  useFocusVizDebugStore.setState({ ...FOCUS_VIZ_DEFAULTS, panelOpen: true });
}

/** Toggle the slider panel — wired to ⌘⇧V from FocusNowPlayingView. */
export function toggleFocusVizDebugPanel(): void {
  useFocusVizDebugStore.setState((s) => ({ panelOpen: !s.panelOpen }));
}
