import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { usePlaybackStore } from "../stores/playbackStore";
import { setSpectrumTilt } from "../lib/commands";
import {
  TUNING_DEFAULTS,
  resetTuning,
  setTuningValue,
  setTuningValues,
  tuningSnapshot,
  useTuning,
  type FocusVizTuning,
} from "./focusVizTuning";

/**
 * Development-only tuning panel for the focus-mode visualiser.
 *
 * `install()` mounts this in its own React root beside the app's, so no
 * production component knows it exists. The panel is toggled with the
 * backtick key while focus mode is open. Bar values go straight to the
 * object the paint loop reads (see ./focusVizTuning); art values are
 * applied by an injected stylesheet that overrides the focus-mode rules,
 * and the panel's own styles are injected the same way, so styles.css
 * carries nothing for this either.
 */

interface ControlSpec {
  key: keyof FocusVizTuning;
  label: string;
  min: number;
  max: number;
  step: number;
  /** Decimal places shown in the readout. */
  digits: number;
}

/**
 * A slider that drives several tuning values at once. `value` reads its
 * position off the current values and `apply` writes every value in
 * `keys`; double-clicking it restores all of them.
 */
interface LinkedSpec {
  label: string;
  min: number;
  max: number;
  step: number;
  digits: number;
  keys: readonly (keyof FocusVizTuning)[];
  value: (t: FocusVizTuning) => number;
  apply: (value: number, t: FocusVizTuning) => void;
}

interface SectionSpec {
  readonly title: string;
  /** Shown above the section's own sliders. */
  readonly linked?: readonly LinkedSpec[];
  readonly controls: readonly ControlSpec[];
}

/**
 * Adds or drops ridge rows at the current spacing: the stack grows or
 * shrinks without its rows closing up, so the scroll speed holds.
 */
const DEPTH_SLIDER: LinkedSpec = {
  label: "Depth",
  min: 2,
  max: 90,
  step: 1,
  digits: 0,
  keys: ["ridgeRows", "ridgeHeight"],
  value: (t) => t.ridgeRows,
  apply: (rows, t) => {
    const spacing = t.ridgeHeight / Math.max(1, t.ridgeRows - 1);
    setTuningValues({ ridgeRows: rows, ridgeHeight: spacing * (rows - 1) });
  },
};

const SECTIONS = [
  {
    title: "Album art",
    controls: [
      { key: "artScale", label: "Scale", min: 0.25, max: 1, step: 0.01, digits: 2 },
      { key: "artOpacity", label: "Opacity", min: 0, max: 1, step: 0.01, digits: 2 },
      { key: "artOffsetY", label: "Vertical", min: -0.5, max: 0.5, step: 0.01, digits: 2 },
      { key: "artColumn", label: "Column (fr)", min: 0.5, max: 2, step: 0.05, digits: 2 },
    ],
  },
  {
    title: "Bars",
    controls: [
      { key: "barMaxHeight", label: "Max height", min: 0.05, max: 1, step: 0.01, digits: 2 },
      { key: "barSpan", label: "Span", min: 0.2, max: 1, step: 0.01, digits: 2 },
      { key: "barGap", label: "Gap px", min: 0, max: 8, step: 0.5, digits: 1 },
      { key: "barAlpha", label: "Alpha", min: 0, max: 1, step: 0.01, digits: 2 },
      { key: "barTipOpacity", label: "Tip opacity", min: 0, max: 1, step: 0.01, digits: 2 },
      { key: "easeAttack", label: "Attack", min: 0.05, max: 1, step: 0.01, digits: 2 },
      { key: "easeDecay", label: "Decay", min: 0.05, max: 1, step: 0.01, digits: 2 },
    ],
  },
  {
    title: "Level curve",
    controls: [
      { key: "gamma", label: "Gamma", min: 0.25, max: 4, step: 0.05, digits: 2 },
      { key: "floorCut", label: "Floor cut", min: 0, max: 0.9, step: 0.01, digits: 2 },
      { key: "gain", label: "Gain", min: 0.5, max: 3, step: 0.05, digits: 2 },
    ],
  },
  {
    title: "Sync",
    controls: [
      { key: "syncLeadMs", label: "Bars lead ms", min: 0, max: 150, step: 1, digits: 0 },
      { key: "ridgeSyncLeadMs", label: "Ridge lead ms", min: 0, max: 150, step: 1, digits: 0 },
    ],
  },
  {
    title: "Ridge",
    linked: [DEPTH_SLIDER],
    controls: [
      { key: "ridgeRows", label: "Rows", min: 2, max: 90, step: 1, digits: 0 },
      { key: "ridgeFullScreenRows", label: "Mobile rows", min: 2, max: 120, step: 1, digits: 0 },
      { key: "ridgeHeight", label: "Stack height", min: 0.05, max: 1.2, step: 0.01, digits: 2 },
      { key: "ridgePeak", label: "Peak", min: 0.02, max: 1, step: 0.01, digits: 2 },
      { key: "ridgeDepthScale", label: "Depth scale", min: 0.1, max: 1.5, step: 0.05, digits: 2 },
      { key: "ridgeBottom", label: "Bottom gap", min: 0, max: 0.4, step: 0.01, digits: 2 },
      { key: "ridgeSpan", label: "Span", min: 0.2, max: 1, step: 0.01, digits: 2 },
      { key: "ridgeLineWidth", label: "Line px", min: 0.5, max: 4, step: 0.25, digits: 2 },
      { key: "ridgeOversample", label: "Oversample", min: 1, max: 8, step: 1, digits: 0 },
      { key: "ridgeAlpha", label: "Alpha", min: 0, max: 1, step: 0.01, digits: 2 },
      { key: "ridgeBackAlpha", label: "Back alpha", min: 0, max: 1, step: 0.01, digits: 2 },
      { key: "ridgeFadeCurve", label: "Fade curve", min: 0.25, max: 4, step: 0.05, digits: 2 },
      { key: "ridgeRowMs", label: "Row ms", min: 16, max: 250, step: 1, digits: 0 },
      { key: "ridgeSpread", label: "Spread", min: 0, max: 3, step: 0.05, digits: 2 },
      { key: "ridgeSmooth", label: "Smooth", min: 0, max: 1, step: 0.01, digits: 2 },
      { key: "ridgeGrain", label: "Grain", min: 0, max: 0.05, step: 0.0005, digits: 4 },
      { key: "ridgeEdgeTaper", label: "Edge taper", min: 0, max: 0.5, step: 0.01, digits: 2 },
      { key: "ridgeAxisCurve", label: "Axis curve", min: 0.4, max: 1.2, step: 0.01, digits: 2 },
      { key: "ridgeAttack", label: "Attack", min: 0.05, max: 1, step: 0.01, digits: 2 },
      { key: "ridgeDecay", label: "Decay", min: 0.05, max: 1, step: 0.01, digits: 2 },
    ],
  },
  {
    title: "Tap",
    controls: [{ key: "tapTilt", label: "Tilt dB/oct", min: -3, max: 6, step: 0.1, digits: 1 }],
  },
  {
    title: "Ridge level curve",
    controls: [
      { key: "ridgeGamma", label: "Gamma", min: 0.25, max: 6, step: 0.05, digits: 2 },
      { key: "ridgeFloorCut", label: "Floor cut", min: 0, max: 0.9, step: 0.01, digits: 2 },
      { key: "ridgeGain", label: "Gain", min: 0.5, max: 3, step: 0.05, digits: 2 },
    ],
  },
] as const satisfies readonly SectionSpec[];

// Every tuning value has a slider: a key added to `FocusVizTuning` without
// a control above fails to compile here, naming the key.
type SliderKey = (typeof SECTIONS)[number]["controls"][number]["key"];
type KeyWithoutSlider = Exclude<keyof FocusVizTuning, SliderKey>;
const EVERY_KEY_HAS_A_SLIDER: [KeyWithoutSlider] extends [never] ? true : KeyWithoutSlider = true;
void EVERY_KEY_HAS_A_SLIDER;

const PANEL_CSS = `
.fv-debug {
  position: fixed;
  top: 44px;
  right: 16px;
  z-index: 600;
  width: 320px;
  max-height: calc(100vh - 60px);
  overflow-y: auto;
  padding: 12px 14px;
  border-radius: 12px;
  background: rgba(10, 10, 14, 0.82);
  border: 1px solid rgba(255, 255, 255, 0.1);
  backdrop-filter: blur(18px);
  -webkit-backdrop-filter: blur(18px);
  color: var(--text-primary);
  font-size: 12px;
  user-select: none;
  -webkit-user-select: none;
}
.fv-debug-head {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 8px;
}
.fv-debug-title {
  flex: 1;
  font-weight: 600;
  letter-spacing: 0.02em;
}
.fv-debug-head button {
  background: rgba(255, 255, 255, 0.08);
  border: none;
  color: var(--text-primary);
  border-radius: 6px;
  padding: 3px 8px;
  font-size: 11px;
  cursor: pointer;
}
.fv-debug-head button:hover {
  background: rgba(255, 255, 255, 0.16);
}
.fv-debug-section {
  margin-top: 8px;
}
.fv-debug-section-title {
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
  color: var(--text-muted);
  margin-bottom: 4px;
}
.fv-debug-row {
  display: grid;
  grid-template-columns: 84px 1fr 44px;
  align-items: center;
  gap: 8px;
  padding: 2px 0;
}
.fv-debug-row input[type="range"] {
  width: 100%;
  margin: 0;
  accent-color: rgb(var(--accent-r, 120), var(--accent-g, 90), var(--accent-b, 220));
}
.fv-debug-label {
  color: var(--text-secondary, var(--text-muted));
}
.fv-debug-row.is-changed .fv-debug-label {
  color: var(--text-primary);
}
.fv-debug-value {
  text-align: right;
  font-variant-numeric: tabular-nums;
  color: var(--text-muted);
}
.fv-debug-row.is-changed .fv-debug-value {
  color: rgb(var(--accent-r, 120), var(--accent-g, 90), var(--accent-b, 220));
}
.fv-debug-hint {
  margin-top: 10px;
  font-size: 10px;
  color: var(--text-muted);
}
.fv-debug.fv-debug-mobile {
  top: 10px;
  left: max(12px, env(safe-area-inset-left));
  right: auto;
  z-index: 10001;
  width: 300px;
  padding: 8px 12px;
}
.fv-debug-mobile .fv-debug-hint {
  margin-top: 4px;
}
.fv-debug-link {
  background: none;
  border: none;
  padding: 0;
  color: inherit;
  font: inherit;
  text-decoration: underline;
}
`;

/**
 * Overrides for the focus-mode art rules in styles.css. The custom
 * property and the two direct properties are the ones the shipped rules
 * set; `!important` keeps these ahead regardless of stylesheet order.
 */
function artCss(t: FocusVizTuning): string {
  return `
.focus-art-container {
  --art-scale: ${t.artScale} !important;
  opacity: ${t.artOpacity} !important;
  transform: translate(-50%, calc(-50% + ${t.artOffsetY * 100}%)) !important;
}
.focus-body {
  grid-template-columns: ${t.artColumn}fr 1fr !important;
}
`;
}

function styleElement(id: string): HTMLStyleElement {
  let el = document.getElementById(id) as HTMLStyleElement | null;
  if (!el) {
    el = document.createElement("style");
    el.id = id;
    document.head.appendChild(el);
  }
  return el;
}

function isEditable(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
}

function Host() {
  const inFocus = usePlaybackStore((s) => s.isFocusMode);
  const mobileVisualizer = usePlaybackStore((s) => s.mobileVisualizerOpen);
  const [open, setOpen] = useState(false);
  const tuning = useTuning();

  // The art overrides stay installed whether or not the panel is showing,
  // so a saved tuning applies as soon as focus mode opens.
  useEffect(() => {
    styleElement("fv-debug-art").textContent = artCss(tuning);
  }, [tuning]);

  // The tap tilt lives in the backend: push it on every change, and once
  // at startup so a saved value applies without opening the panel.
  useEffect(() => {
    setSpectrumTilt(tuning.tapTilt).catch(() => {});
  }, [tuning.tapTilt]);

  useEffect(() => {
    if (!inFocus) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "`" || e.metaKey || e.ctrlKey || e.altKey || isEditable(e.target)) return;
      e.preventDefault();
      setOpen((v) => !v);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [inFocus]);

  if (mobileVisualizer) return <MobilePanel tuning={tuning} />;
  if (!inFocus || !open) return null;
  return <Panel tuning={tuning} onClose={() => setOpen(false)} />;
}

/**
 * The touch-screen counterpart, shown while the mobile full-screen
 * visualiser is open: its depth (`ridgeFullScreenRows`, at the usual row
 * spacing) pinned to a corner above the overlay. It renders in this root,
 * outside the overlay's tree, so a drag on it never reaches the overlay's
 * tap-to-close.
 */
function MobilePanel({ tuning }: { tuning: FocusVizTuning }) {
  const key = "ridgeFullScreenRows";
  const rows = tuning[key];
  const stack = (tuning.ridgeHeight * (rows - 1)) / Math.max(1, tuning.ridgeRows - 1);
  return (
    <div className="fv-debug fv-debug-mobile" onClick={(e) => e.stopPropagation()}>
      <Slider
        label="Depth"
        min={2}
        max={120}
        step={1}
        digits={0}
        value={rows}
        changed={rows !== TUNING_DEFAULTS[key]}
        onChange={(v) => setTuningValue(key, v)}
        onReset={() => setTuningValue(key, TUNING_DEFAULTS[key])}
      />
      <div className="fv-debug-hint">
        stack height {stack.toFixed(3)} · double-tap resets ·{" "}
        <button type="button" className="fv-debug-link" onClick={resetTuning}>
          reset all
        </button>
      </div>
    </div>
  );
}

function Slider({
  label,
  min,
  max,
  step,
  digits,
  value,
  changed,
  onChange,
  onReset,
}: {
  label: string;
  min: number;
  max: number;
  step: number;
  digits: number;
  value: number;
  changed: boolean;
  onChange: (value: number) => void;
  onReset: () => void;
}) {
  return (
    <label className={`fv-debug-row${changed ? " is-changed" : ""}`}>
      <span className="fv-debug-label">{label}</span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        onDoubleClick={onReset}
      />
      <span className="fv-debug-value">{value.toFixed(digits)}</span>
    </label>
  );
}

const SECTION_LIST: readonly SectionSpec[] = SECTIONS;

function Panel({ tuning, onClose }: { tuning: FocusVizTuning; onClose: () => void }) {
  const [copied, setCopied] = useState(false);

  const copy = () => {
    const json = JSON.stringify(tuningSnapshot(), null, 2);
    console.log(`[focus-viz tuning]\n${json}`);
    navigator.clipboard
      ?.writeText(json)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1200);
      })
      .catch(() => {});
  };

  return (
    <div
      className="fv-debug"
      // Keys pressed inside the panel (arrows on a slider, space) must not
      // reach the app-wide shortcuts.
      onKeyDown={(e) => e.stopPropagation()}
    >
      <div className="fv-debug-head">
        <span className="fv-debug-title">Visualiser tuning</span>
        <button type="button" onClick={copy}>
          {copied ? "Copied" : "Copy JSON"}
        </button>
        <button type="button" onClick={resetTuning}>
          Reset
        </button>
        <button type="button" onClick={onClose} aria-label="Close">
          ×
        </button>
      </div>
      {SECTION_LIST.map((section) => (
        <div className="fv-debug-section" key={section.title}>
          <div className="fv-debug-section-title">{section.title}</div>
          {section.linked?.map((l) => (
            <Slider
              key={l.label}
              label={l.label}
              min={l.min}
              max={l.max}
              step={l.step}
              digits={l.digits}
              value={l.value(tuning)}
              changed={l.keys.some((k) => tuning[k] !== TUNING_DEFAULTS[k])}
              onChange={(v) => l.apply(v, tuning)}
              onReset={() =>
                setTuningValues(Object.fromEntries(l.keys.map((k) => [k, TUNING_DEFAULTS[k]])))
              }
            />
          ))}
          {section.controls.map((c) => (
            <Slider
              key={c.key}
              label={c.label}
              min={c.min}
              max={c.max}
              step={c.step}
              digits={c.digits}
              value={tuning[c.key]}
              changed={tuning[c.key] !== TUNING_DEFAULTS[c.key]}
              onChange={(v) => setTuningValue(c.key, v)}
              onReset={() => setTuningValue(c.key, TUNING_DEFAULTS[c.key])}
            />
          ))}
        </div>
      ))}
      <div className="fv-debug-hint">` toggles · double-click a slider to reset it</div>
    </div>
  );
}

/** Mount the panel host in its own root and inject its styles. */
export function install(): void {
  if (document.getElementById("fv-debug-root")) return;
  styleElement("fv-debug-panel").textContent = PANEL_CSS;
  const host = document.createElement("div");
  host.id = "fv-debug-root";
  document.body.appendChild(host);
  createRoot(host).render(<Host />);
}
