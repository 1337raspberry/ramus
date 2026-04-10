/* =============================================================================
 * DEBUG (focus visualiser panel)
 *
 * DELETE THIS ENTIRE FILE when we're happy that the visualiser tuning in
 * `VISUALIZER_DEFAULTS` is locked in. The baked-in values in the store
 * survive this deletion — only the live tuning UI goes away.
 *
 * See the removal guide at the top of `ui/src/stores/visualizerDebugStore.ts`
 * for the full checklist of things to drop alongside this component.
 * ============================================================================= */

import { useState } from "react";
import {
  useVisualizerDebugStore,
  VISUALIZER_DEFAULTS,
  type VisualizerParams,
} from "../stores/visualizerDebugStore";
import { IconClose } from "./Icons";

interface SliderDef {
  key: keyof VisualizerParams;
  label: string;
  min: number;
  max: number;
  step: number;
}

interface Group {
  name: string;
  sliders: SliderDef[];
}

/**
 * Slider groups for the focus visualiser debug panel. Mirrors the layout
 * of VisualizerParams — keep in sync when adding new tunable values.
 */
const GROUPS: Group[] = [
  {
    name: "Shape",
    sliders: [
      { key: "pointCount", label: "Points", min: 8, max: 128, step: 1 },
      { key: "shapePower", label: "Shape power", min: 0.3, max: 3, step: 0.05 },
    ],
  },
  {
    name: "Octave weights",
    sliders: [
      { key: "weightA", label: "Weight A (low)", min: 0, max: 1, step: 0.01 },
      { key: "weightB", label: "Weight B (mid)", min: 0, max: 1, step: 0.01 },
      { key: "weightC", label: "Weight C (high)", min: 0, max: 1, step: 0.01 },
    ],
  },
  {
    name: "Spatial frequency",
    sliders: [
      { key: "spatialA", label: "Spatial A", min: 0.05, max: 5, step: 0.05 },
      { key: "spatialB", label: "Spatial B", min: 0.05, max: 5, step: 0.05 },
      { key: "spatialC", label: "Spatial C", min: 0.05, max: 5, step: 0.05 },
    ],
  },
  {
    name: "Temporal rate",
    sliders: [
      { key: "phaseRateA", label: "Phase rate A", min: 0, max: 0.005, step: 0.00005 },
      { key: "phaseRateB", label: "Phase rate B", min: 0, max: 0.005, step: 0.00005 },
      { key: "phaseRateC", label: "Phase rate C", min: 0, max: 0.005, step: 0.00005 },
    ],
  },
  {
    name: "Drooping V bias",
    sliders: [
      { key: "centerPower", label: "Center power", min: 0.5, max: 4, step: 0.05 },
      { key: "centerStrength", label: "Edge drop", min: 0, max: 1, step: 0.01 },
    ],
  },
  {
    name: "Music response",
    sliders: [
      { key: "musicDepth", label: "Music depth", min: 0, max: 2, step: 0.01 },
      { key: "shapeMixBase", label: "Shape mix base", min: 0, max: 1, step: 0.01 },
      { key: "shapeMixRange", label: "Shape mix range", min: 0, max: 1, step: 0.01 },
      { key: "idleWhileMusic", label: "Idle under music", min: 0, max: 1, step: 0.01 },
    ],
  },
  {
    name: "Idle (silent)",
    sliders: [
      { key: "idleBase", label: "Idle base", min: 0, max: 0.2, step: 0.005 },
      { key: "idleAmplitude", label: "Idle amplitude", min: 0, max: 0.2, step: 0.005 },
    ],
  },
  {
    name: "Animation",
    sliders: [
      { key: "attackEase", label: "Attack", min: 0.01, max: 1, step: 0.01 },
      { key: "decayEase", label: "Decay", min: 0.01, max: 1, step: 0.01 },
    ],
  },
  {
    name: "Timing",
    sliders: [{ key: "visualDelayMs", label: "Visual delay (ms)", min: 0, max: 1000, step: 5 }],
  },
  {
    name: "Appearance",
    sliders: [
      { key: "fillOpacity", label: "Fill opacity", min: 0, max: 1, step: 0.01 },
      { key: "strokeOpacity", label: "Stroke opacity", min: 0, max: 1, step: 0.01 },
      { key: "strokeWidth", label: "Stroke width", min: 0.5, max: 6, step: 0.1 },
      { key: "maxDepthPct", label: "Max depth %", min: 5, max: 100, step: 0.5 },
    ],
  },
];

/** Format a slider value for display, picking precision from its step. */
function formatDisplay(value: number, step: number): string {
  if (step >= 1) return value.toFixed(0);
  if (step >= 0.1) return value.toFixed(2);
  if (step >= 0.01) return value.toFixed(2);
  if (step >= 0.001) return value.toFixed(3);
  return value.toFixed(5);
}

/** Format a value for the "Copy" output — slightly more precision than display. */
function formatForCopy(value: number, step: number): string {
  if (step >= 1) return String(Math.round(value));
  if (step >= 0.01) return value.toFixed(3);
  if (step >= 0.001) return value.toFixed(4);
  return value.toFixed(6);
}

/** Look up the step for a given key by walking the slider definitions. */
function stepFor(key: keyof VisualizerParams): number {
  for (const group of GROUPS) {
    const slider = group.sliders.find((s) => s.key === key);
    if (slider) return slider.step;
  }
  return 0.01;
}

export default function FocusVisualizerDebugPanel() {
  const params = useVisualizerDebugStore();
  const [copyState, setCopyState] = useState<"idle" | "copied" | "error">("idle");

  const handleCopy = () => {
    const lines = (Object.keys(VISUALIZER_DEFAULTS) as (keyof VisualizerParams)[]).map((key) => {
      const value = params[key];
      const formatted = formatForCopy(value, stepFor(key));
      return `  ${key}: ${formatted},`;
    });
    const text = `export const VISUALIZER_DEFAULTS: VisualizerParams = {\n${lines.join("\n")}\n};`;
    navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopyState("copied");
        setTimeout(() => setCopyState("idle"), 1500);
      })
      .catch(() => {
        setCopyState("error");
        setTimeout(() => setCopyState("idle"), 1500);
      });
  };

  return (
    <div className="focus-viz-debug" onClick={(e) => e.stopPropagation()}>
      <div className="focus-viz-debug-header">
        <span className="focus-viz-debug-title">Visualiser tuning</span>
        <div className="focus-viz-debug-actions">
          <button className="focus-viz-debug-btn" onClick={params.reset} title="Reset to defaults">
            Reset
          </button>
          <button className="focus-viz-debug-btn" onClick={handleCopy} title="Copy current values">
            {copyState === "copied" ? "Copied!" : copyState === "error" ? "Failed" : "Copy"}
          </button>
          <button
            className="focus-viz-debug-close"
            onClick={params.togglePanel}
            title="Close (⇧⌘V)"
          >
            <IconClose size={12} />
          </button>
        </div>
      </div>
      <div className="focus-viz-debug-body">
        {GROUPS.map((group) => (
          <div key={group.name} className="focus-viz-debug-group">
            <div className="focus-viz-debug-group-name">{group.name}</div>
            {group.sliders.map((slider) => {
              const value = params[slider.key];
              return (
                <label key={slider.key} className="focus-viz-debug-slider">
                  <div className="focus-viz-debug-slider-row">
                    <span className="focus-viz-debug-slider-label">{slider.label}</span>
                    <span className="focus-viz-debug-slider-value">
                      {formatDisplay(value, slider.step)}
                    </span>
                  </div>
                  <input
                    type="range"
                    min={slider.min}
                    max={slider.max}
                    step={slider.step}
                    value={value}
                    onChange={(e) => params.setParam(slider.key, Number(e.target.value))}
                  />
                </label>
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}
