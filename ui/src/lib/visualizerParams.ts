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
  /**
   * How far ahead of the audible position the bars read their frame, in
   * ms: what a paint takes to reach the screen, plus the half frame an
   * onset waits for the next frame, plus the paints the attack easing
   * needs to lift a bar halfway. Each band's own filter lag comes from
   * the backend on top of this (`get_spectrum_layout`).
   */
  syncLeadMs: number;

  /** Rows in the ridge stack, the live front row included. */
  ridgeRows: number;
  /**
   * Rows on the mobile full-screen visualiser, which has no controls to
   * leave room for. The rows keep the spacing above (`ridgeHeight` over
   * `ridgeRows - 1`), so the stack reaches further back and scrolls at the
   * same speed.
   */
  ridgeFullScreenRows: number;
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
  /**
   * Points each ridge row is resampled to per band, along a monotone
   * cubic through the bands (1..8). 1 draws straight lines between the
   * bands themselves.
   */
  ridgeOversample: number;
  /** Line alpha at the front row. */
  ridgeAlpha: number;
  /** Line alpha at the back row. */
  ridgeBackAlpha: number;
  /**
   * Shape of the alpha fade from the front row to the back: 1 is linear,
   * below 1 holds the brightness further back, above 1 tails off
   * gradually toward the back.
   */
  ridgeFadeCurve: number;
  /** Wall-clock interval between history rows, in ms. */
  ridgeRowMs: number;
  /**
   * Width in points (a bell's sigma) each ridge peak is spread into
   * without losing height; 0 is identity. Applied before `ridgeSmooth`.
   */
  ridgeSpread: number;
  /** Neighbour blend applied to each ridge point, 0..1; 0 is identity. */
  ridgeSmooth: number;
  /**
   * Random texture on every resampled point of a ridge row as a fraction
   * of its own height, re-rolled for every history row; 0 is off.
   */
  ridgeGrain: number;
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
  /** As `syncLeadMs`, for the ridge and its own attack easing. */
  ridgeSyncLeadMs: number;
  /**
   * Exponent on the ridge's frequency axis. A band's position along the
   * log-spaced range, 0 at the lowest and 1 at the highest, is drawn at
   * that position raised to this power across the width: 1 is the plain
   * log axis, and below 1 gives the bass and low mids more of the width
   * and the top octaves less. Music's energy sits low on a log axis, so
   * the body of a mix otherwise crowds the left edge while the right
   * third only moves for cymbals and sibilance.
   */
  ridgeAxisCurve: number;
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
  // About 32 ms to the screen, 8 ms of frame wait and two paints (the
  // attack takes two to pass half height).
  syncLeadMs: 60,

  // Row spacing (height / (rows - 1)) and the row interval together set
  // how fast the stack scrolls, so the rows, height and back-row scale
  // change together: the stack runs 1.7 s deep with its rows the same
  // distance apart as a shallower one, and the fade below carries the
  // back of it out to nothing.
  ridgeRows: 51,
  ridgeFullScreenRows: 67,
  ridgeHeight: 0.621,
  ridgePeak: 0.6,
  ridgeDepthScale: 0.394,
  ridgeBottom: 0.01,
  ridgeSpan: 1,
  ridgeLineWidth: 1.25,
  ridgeOversample: 5,
  // Same wide-gamut allowance as the bars.
  ridgeAlpha: isHDR ? 0.6 : 0.85,
  ridgeBackAlpha: 0,
  ridgeFadeCurve: 0.75,
  ridgeRowMs: 33,
  ridgeSpread: 1,
  ridgeSmooth: 0.1,
  ridgeGrain: 0.03,
  ridgeEdgeTaper: 0.22,
  ridgeAttack: 0.67,
  ridgeDecay: 0.5,
  ridgeGamma: 4,
  ridgeFloorCut: 0.6,
  ridgeGain: 1.15,
  // As `syncLeadMs`; the ridge's attack passes half height in one paint.
  ridgeSyncLeadMs: 45,
  // Moves 100 Hz..1 kHz about a tenth of the width to the right, most
  // around 300 Hz; the ends stay put.
  ridgeAxisCurve: 0.75,
};
