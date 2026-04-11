// DEBUG (focus viz tuning) — entire file is debug scaffolding.
//
// Slider panel for live-tuning the focus-mode FFT visualiser.
// Toggled with ⌘⇧V (Cmd/Ctrl+Shift+V) while in focus mode — handler
// lives in FocusNowPlayingView.tsx. Anchored to the bottom centre of
// the window so it doesn't cover the album art / track metadata.
//
// To remove the whole debug system, see the checklist at the top of
// `ui/src/stores/focusVizDebugStore.ts`.

import {
  FOCUS_VIZ_DEFAULTS,
  resetFocusVizDebug,
  setFocusVizParam,
  toggleFocusVizDebugPanel,
  useFocusVizDebugStore,
  type FocusVizDebugState,
} from "../stores/focusVizDebugStore";
import { IconClose } from "./Icons";

interface SliderSpec {
  key: keyof FocusVizDebugState;
  label: string;
  min: number;
  max: number;
  step: number;
  unit?: string;
}

const SLIDER_GROUPS: { name: string; sliders: SliderSpec[] }[] = [
  {
    name: "Layout",
    sliders: [
      { key: "barMaxHeightPct", label: "Max bar height", min: 5, max: 80, step: 1, unit: "%" },
      { key: "barGapPx", label: "Bar gap", min: 0, max: 8, step: 0.5, unit: "px" },
      {
        key: "minVisibleHeightPx",
        label: "Min visible bar",
        min: 0,
        max: 3,
        step: 0.1,
        unit: "px",
      },
      {
        key: "placeholderTopVh",
        label: "“Analysing” top offset",
        min: 0,
        max: 15,
        step: 0.25,
        unit: "vh",
      },
    ],
  },
  {
    name: "Animation",
    sliders: [
      { key: "easeAttack", label: "Attack ease", min: 0.05, max: 1.0, step: 0.01 },
      { key: "easeDecay", label: "Decay ease", min: 0.05, max: 1.0, step: 0.01 },
    ],
  },
  {
    name: "Colour & fill",
    sliders: [
      { key: "gradientTopOpacity", label: "Top opacity", min: 0, max: 1, step: 0.01 },
      { key: "gradientBottomOpacity", label: "Bottom opacity", min: 0, max: 1, step: 0.01 },
      { key: "globalAlpha", label: "Global alpha", min: 0, max: 1, step: 0.01 },
      { key: "borderWidthPx", label: "Border width", min: 0, max: 3, step: 0.5, unit: "px" },
      { key: "borderOpacity", label: "Border opacity", min: 0, max: 1, step: 0.01 },
    ],
  },
  {
    name: "Bass decorrelation",
    sliders: [
      { key: "bassNoiseAmount", label: "Noise amount", min: 0, max: 0.3, step: 0.005 },
      { key: "bassNoiseBandCutoff", label: "Band cutoff", min: 0, max: 96, step: 1 },
      { key: "bassNoiseGate", label: "Silent-bar gate", min: 0, max: 0.3, step: 0.005 },
    ],
  },
  {
    name: "Line mode",
    sliders: [
      {
        key: "lineSmoothingWindow",
        label: "Smoothing window",
        min: 1,
        max: 40,
        step: 1,
        unit: "bars",
      },
    ],
  },
];

export default function FocusVisualizerDebugPanel() {
  const open = useFocusVizDebugStore((s) => s.panelOpen);
  if (!open) return null;
  return (
    <div className="focus-viz-debug" role="dialog" aria-label="Focus visualiser tuning (debug)">
      <div className="focus-viz-debug-header">
        <span className="focus-viz-debug-title">Focus visualiser tuning · debug</span>
        <div className="focus-viz-debug-actions">
          <button
            type="button"
            className="focus-viz-debug-btn"
            onClick={resetFocusVizDebug}
            title="Reset all to defaults"
          >
            Reset
          </button>
          <button
            type="button"
            className="focus-viz-debug-close"
            onClick={toggleFocusVizDebugPanel}
            title="Close (⌘⇧V)"
          >
            <IconClose />
          </button>
        </div>
      </div>
      <div className="focus-viz-debug-body">
        {SLIDER_GROUPS.map((group) => (
          <div className="focus-viz-debug-group" key={group.name}>
            <div className="focus-viz-debug-group-name">{group.name}</div>
            {group.sliders.map((spec) => (
              <SliderRow key={spec.key} spec={spec} />
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

function SliderRow({ spec }: { spec: SliderSpec }) {
  const value = useFocusVizDebugStore((s) => s[spec.key]) as number;
  const def = FOCUS_VIZ_DEFAULTS[spec.key] as number;
  const isModified = value !== def;
  // Show 2 decimal places for fine-grained sliders, 0 for whole-number ones.
  const decimals = spec.step >= 1 ? 0 : spec.step >= 0.1 ? 1 : 2;
  return (
    <div className="focus-viz-debug-slider">
      <div className="focus-viz-debug-slider-row">
        <span className="focus-viz-debug-slider-label">{spec.label}</span>
        <span className={`focus-viz-debug-slider-value${isModified ? " is-modified" : ""}`}>
          {value.toFixed(decimals)}
          {spec.unit ?? ""}
        </span>
      </div>
      <input
        type="range"
        min={spec.min}
        max={spec.max}
        step={spec.step}
        value={value}
        onChange={(e) => setFocusVizParam(spec.key, parseFloat(e.target.value))}
      />
    </div>
  );
}
