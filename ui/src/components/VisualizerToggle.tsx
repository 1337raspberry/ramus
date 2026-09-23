import { usePlaybackStore } from "../stores/playbackStore";
import { useSettingsStore } from "../stores/settingsStore";
import {
  nextVisualizerMode,
  shownVisualizerMode,
  type VisualizerMode,
} from "../lib/visualizerMode";
import { IconWave, IconRidge, IconWaveOff } from "./Icons";

const LABEL: Record<VisualizerMode, string> = {
  ridge: "pulsar",
  bars: "bars",
  off: "off",
};

const ICON: Record<VisualizerMode, typeof IconWave> = {
  ridge: IconRidge,
  bars: IconWave,
  off: IconWaveOff,
};

/**
 * The visualiser mode button: one click steps to the next mode, and the
 * icon is the mode on screen. Shared by the focus track row and the
 * clear-screen corner player so a new mode lands in both at once; the
 * clear screen's cycle leaves out off (`lib/visualizerMode.ts`). Absent
 * while the visualiser is disabled in settings, since there is nothing
 * for it to switch.
 */
export default function VisualizerToggle() {
  const mode = usePlaybackStore((s) => s.visualizerMode);
  const clear = usePlaybackStore((s) => s.focusClear);
  const toggle = usePlaybackStore((s) => s.toggleVisualizer);
  const disabled = useSettingsStore((s) => s.disableSpectrum);
  if (disabled) return null;
  const shown = shownVisualizerMode(mode, clear);
  const Icon = ICON[shown];
  const title = `Visualiser: ${LABEL[shown]} — click for ${LABEL[nextVisualizerMode(mode, clear)]}`;
  return (
    <button className="np-viz-btn" onClick={toggle} title={title} aria-label={title}>
      <Icon />
    </button>
  );
}
