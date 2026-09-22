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
 *
 * The `ridge*` values do the same for the ridgeline mode
 * (`lib/ridgeline.ts`): its own layout, and its own level curve and
 * easing, because a scrolling line wants slower dynamics than a bar.
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

  /** Rows in the ridge stack, the live front row included. */
  ridgeRows: number;
  /** Height of the stack (front baseline to back baseline) as a fraction of the window height. */
  ridgeHeight: number;
  /** Full-scale displacement on the front row as a fraction of the window height. */
  ridgePeak: number;
  /** Peak scale of the back row relative to the front; 1 is flat. */
  ridgeDepthScale: number;
  /** Gap between the front baseline and the bottom edge as a fraction of the window height. */
  ridgeBottom: number;
  /** Width of the ridge as a fraction of the window width, centred. */
  ridgeSpan: number;
  /** Ridge stroke width in CSS px. */
  ridgeLineWidth: number;
  /** Line alpha at the front row. */
  ridgeAlpha: number;
  /** Line alpha at the back row. */
  ridgeBackAlpha: number;
  /** Wall-clock interval between history rows, in ms. */
  ridgeRowMs: number;
  /** Neighbour blend applied to each ridge point, 0..1; 0 is identity. */
  ridgeSmooth: number;
  /** Fraction of the ridge width over which each end tapers to the baseline. */
  ridgeEdgeTaper: number;
  /** Easing factor per 60 Hz frame while a ridge point rises. */
  ridgeAttack: number;
  /** Easing factor per 60 Hz frame while a ridge point falls. */
  ridgeDecay: number;
  /** Ridge level-curve exponent; 1 is identity. */
  ridgeGamma: number;
  /** Ridge level-curve threshold, 0..1; 0 is identity. */
  ridgeFloorCut: number;
  /** Ridge level-curve multiplier; 1 is identity. */
  ridgeGain: number;
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

  ridgeRows: 28,
  ridgeHeight: 0.34,
  ridgePeak: 0.6,
  ridgeDepthScale: 0.6,
  ridgeBottom: 0.01,
  ridgeSpan: 1,
  ridgeLineWidth: 1.25,
  // Same wide-gamut allowance as the bars.
  ridgeAlpha: isHDR ? 0.6 : 0.85,
  ridgeBackAlpha: isHDR ? 0.15 : 0.25,
  ridgeRowMs: 33,
  ridgeSmooth: 0.19,
  ridgeEdgeTaper: 0.22,
  ridgeAttack: 0.67,
  ridgeDecay: 0.29,
  ridgeGamma: 4,
  ridgeFloorCut: 0.67,
  ridgeGain: 1.15,
};
