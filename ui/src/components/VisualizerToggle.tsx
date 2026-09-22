import { usePlaybackStore, type VisualizerMode } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import { IconWave, IconRidge } from "./Icons";

/** Tooltip per visualiser mode; the button steps to the next mode. */
const TITLE: Record<VisualizerMode, string> = {
  bars: "Visualiser: bars — click for ridgeline",
  ridge: "Visualiser: ridgeline — click for bars",
};

/**
 * The visualiser mode button: one click steps to the next mode, and the
 * icon is the mode on screen. Shared by the focus track row and the
 * clear-screen corner player so a new mode lands in both at once. Absent
 * while the visualiser is disabled in settings, since there is nothing
 * for it to switch.
 */
export default function VisualizerToggle() {
  const mode = usePlaybackStore((s) => s.visualizerMode);
  const toggle = usePlaybackStore((s) => s.toggleVisualizer);
  const disabled = useSettingsStore((s) => s.disableSpectrum);
  if (disabled) return null;
  return (
    <button className="np-viz-btn" onClick={toggle} title={TITLE[mode]} aria-label={TITLE[mode]}>
      {mode === "ridge" ? <IconRidge /> : <IconWave />}
    </button>
  );
}
