import { isHDR } from "./hdr";

/**
 * Focus-mode visualiser parameters: bar geometry, opacity, easing and the
 * display-side level curve. `FocusVisualizer` reads this object on every
 * paint. The values are the shipped look; the object is deliberately
 * mutable so development tooling can adjust them live.
 *
 * The level curve reshapes each bar's 0..1 value before easing. The
 * backend already maps a band's dB position (inside a sliding 55 dB
 * window under a running peak) through a 0.6 power curve; on top of that
 * `gamma` above 1 pushes quiet bars down and stretches loud ones,
 * `floorCut` zeroes everything below it and rescales the rest to full
 * range, and `gain` multiplies last with the result clamped to 1.
 */
export interface VisualizerParams {
  /** Tallest bar as a fraction of the window height. */
  barMaxHeight: number;
  /** Width of the bar field as a fraction of the window width, centred. */
  barSpan: number;
  /** Gap between neighbouring bars in CSS px. */
  barGap: number;
  /** Alpha the whole bar layer is drawn at. */
  barAlpha: number;
  /** Gradient opacity at the bar tips; the root is fixed at 0.95. */
  barTipOpacity: number;
  /** Easing factor per 60 Hz frame while a bar rises. */
  easeAttack: number;
  /** Easing factor per 60 Hz frame while a bar falls. */
  easeDecay: number;
  /** Level-curve exponent; 1 is identity. */
  gamma: number;
  /** Level-curve threshold, 0..1; 0 is identity. */
  floorCut: number;
  /** Level-curve multiplier; 1 is identity. */
  gain: number;
}

export const VISUALIZER_PARAMS: VisualizerParams = {
  barMaxHeight: 0.15,
  barSpan: 1,
  barGap: 2,
  // A wide-gamut panel renders the accent far brighter, so the bars are
  // drawn fainter there than on an SDR display.
  barAlpha: isHDR ? 0.3 : 0.45,
  barTipOpacity: isHDR ? 0.3 : 0.45,
  easeAttack: 0.31,
  easeDecay: 0.65,
  gamma: 2.8,
  floorCut: 0,
  gain: 1.1,
};
