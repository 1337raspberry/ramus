import { create } from "zustand";

/* =============================================================================
 * FOCUS VISUALISER — DEBUG TUNING STORE
 *
 * The values in `VISUALIZER_DEFAULTS` below are the live, tuned defaults the
 * visualiser uses at runtime. They are considered permanent and should NOT
 * be deleted when the debug panel is removed — only the debug panel wiring
 * (`useVisualizerDebugStore`, `panelOpen`, `togglePanel`, `setParam`,
 * `reset`) is disposable.
 *
 * --- Removal guide (when we're happy the tuning is locked in) -----------------
 *
 *   1. Delete `ui/src/components/FocusVisualizerDebugPanel.tsx`
 *   2. In `ui/src/App.tsx`, remove the lines flagged with
 *      `// DEBUG (focus visualiser panel):` — the import, the store selector,
 *      the Cmd+Shift+V shortcut branch, and the `<FocusVisualizerDebugPanel />`
 *      render.
 *   3. In `ui/src/stores/playbackStore.ts`, replace the `onAudioLevel` block
 *      flagged with `// DEBUG (focus visualiser panel):` with a simple
 *      `setTimeout(() => set({ audioLevels: payload }), VISUAL_DELAY_MS)`
 *      where `VISUAL_DELAY_MS` is the `visualDelayMs` value from
 *      `VISUALIZER_DEFAULTS` (currently 600).
 *   4. In `ui/src/components/FocusVisualizer.tsx`, replace the
 *      `useVisualizerDebugStore.getState()` call (flagged with
 *      `// DEBUG (focus visualiser panel):`) with a direct
 *      `import { VISUALIZER_DEFAULTS } from "../stores/visualizerDebugStore"`
 *      and use that object. Or inline the constants into the component.
 *   5. Reduce THIS file to just the `VisualizerParams` interface and the
 *      `VISUALIZER_DEFAULTS` export. Delete everything from
 *      `VisualizerDebugState` downward.
 *
 * Everything above step 5 keeps the visualiser reading a single canonical
 * set of values so the runtime behaviour is unchanged after removal.
 * ============================================================================= */

/**
 * Tunable parameters for the focus-mode visualiser. The `FocusVisualizer`
 * canvas component reads these from the store every frame via `getState()`
 * (no React subscription on the hot path), so changes from the debug panel
 * take effect immediately without forcing re-renders on every animation
 * tick.
 *
 * If you tune these via the debug panel and want to make them permanent,
 * hit "Copy" in the panel header and paste the resulting object literal
 * over `VISUALIZER_DEFAULTS` below.
 */
export interface VisualizerParams {
  // --- Shape ---
  pointCount: number;
  shapePower: number;

  // --- Noise octaves (weights should roughly sum to 1) ---
  weightA: number;
  weightB: number;
  weightC: number;

  // --- Spatial frequencies: higher = neighbours decorrelate faster ---
  spatialA: number;
  spatialB: number;
  spatialC: number;

  // --- Temporal phase rates: how fast each octave evolves over time ---
  phaseRateA: number;
  phaseRateB: number;
  phaseRateC: number;

  // --- Drooping-V center bias ---
  centerPower: number;
  centerStrength: number;

  // --- Music response ---
  musicDepth: number;
  shapeMixBase: number;
  shapeMixRange: number;
  idleWhileMusic: number;

  // --- Idle behaviour when silent ---
  idleBase: number;
  idleAmplitude: number;

  // --- Animation spring ---
  attackEase: number;
  decayEase: number;

  // --- Timing compensation ---
  // `astats` measures PCM samples inside mpv's filter chain — upstream of
  // the actual audio output, so the visualiser naturally leads the speakers
  // by approximately the mpv `audio-buffer` setting (~500 ms by default).
  // This value introduces a matching frontend delay on `audio-level` events
  // so the visual reaction lines up with what you actually hear. 0 = no
  // delay (pure realtime data, expect visual lead).
  visualDelayMs: number;

  // --- Appearance ---
  fillOpacity: number;
  strokeOpacity: number;
  strokeWidth: number;
  // Max curve drape as a percentage of the full focus window. The
  // visualiser is rendered as a full-window background layer, and the
  // curve's deepest possible point is this fraction of the canvas height.
  maxDepthPct: number;
}

/**
 * Baked-in tuned values from manual debug-panel tuning. PERMANENT — these
 * survive removal of the debug panel and are the single source of truth
 * for the visualiser at runtime. Only re-tune via the debug panel (⇧⌘V in
 * focus mode) and paste over this block when you want to make new values
 * permanent.
 */
export const VISUALIZER_DEFAULTS: VisualizerParams = {
  pointCount: 12,
  shapePower: 1.35,
  weightA: 0.55,
  weightB: 0.3,
  weightC: 0.15,
  spatialA: 0.6,
  spatialB: 1.5,
  spatialC: 2.8,
  phaseRateA: 0.00265,
  phaseRateB: 0.00145,
  phaseRateC: 0.0022,
  centerPower: 2.4,
  centerStrength: 0.34,
  musicDepth: 1.42,
  shapeMixBase: 0.52,
  shapeMixRange: 0.73,
  idleWhileMusic: 0.0,
  idleBase: 0.0,
  idleAmplitude: 0.0,
  attackEase: 0.18,
  decayEase: 0.28,
  visualDelayMs: 600,
  fillOpacity: 0.14,
  strokeOpacity: 0.1,
  strokeWidth: 1.9,
  maxDepthPct: 100.0,
};

// DEBUG (focus visualiser panel): everything below this line is disposable.
// When the debug panel is removed, delete from here to the end of the file
// and have `FocusVisualizer`, `playbackStore.onAudioLevel`, and any other
// consumers import `VISUALIZER_DEFAULTS` directly instead of calling
// `useVisualizerDebugStore.getState()`. See removal guide at the top of
// this file.
interface VisualizerDebugState extends VisualizerParams {
  panelOpen: boolean;
  togglePanel: () => void;
  setParam: <K extends keyof VisualizerParams>(key: K, value: VisualizerParams[K]) => void;
  reset: () => void;
}

export const useVisualizerDebugStore = create<VisualizerDebugState>((set) => ({
  ...VISUALIZER_DEFAULTS,
  panelOpen: false,
  togglePanel: () => set((s) => ({ panelOpen: !s.panelOpen })),
  setParam: (key, value) => set({ [key]: value } as Partial<VisualizerDebugState>),
  reset: () => set({ ...VISUALIZER_DEFAULTS }),
}));
