// Ported from ramusTV `RamusTV/Visualiser/RidgeParams.swift`.

import Foundation

/// Ridge layout, level curve, easing and sync values.
///
/// Ported from ramus `ui/src/lib/visualizerParams.ts` (`VISUALIZER_PARAMS`,
/// the `ridge*` fields, SDR values). Names match the source so the two stay
/// easy to compare.
///
/// The level curve reshapes each point's 0...1 value before easing:
/// `ridgeFloorCut` zeroes everything at or below it and rescales the rest to
/// full range, `ridgeGamma` above 1 pushes quiet points down and stretches
/// loud ones, and `ridgeGain` multiplies last with the result clamped to 1.
///
/// Row spacing (`ridgeHeight / (ridgeRows - 1)`) and `ridgeRowMs` together
/// set how fast the stack scrolls, so the rows, height and back-row scale
/// change together: the stack runs 1.7 s deep, and the fade carries the back
/// of it out to nothing.
struct RidgeParams: Equatable {
    /// Rows in the stack, the live front row included.
    var ridgeRows = 51
    /// Rows on a full-screen stack with no controls to leave room for. The
    /// rows keep the spacing above (`ridgeHeight` over `ridgeRows - 1`), so
    /// the stack reaches further back and scrolls at the same speed.
    var ridgeFullScreenRows = 67
    /// Height of the stack (front baseline to back baseline) as a fraction of
    /// the frame height.
    var ridgeHeight = 0.621
    /// Full-scale displacement on the front row as a fraction of the frame height.
    var ridgePeak = 0.6
    /// Peak scale of the back row relative to the front; 1 is flat.
    var ridgeDepthScale = 0.394
    /// Gap between the front baseline and the bottom edge as a fraction of
    /// the frame height.
    var ridgeBottom = 0.01
    /// Width of the ridge as a fraction of the frame width, centred.
    var ridgeSpan = 1.0
    /// Stroke width in points.
    var ridgeLineWidth = 1.25
    /// Points each row is resampled to per band, along a monotone cubic
    /// through the bands (1...8). 1 draws straight lines between the bands.
    var ridgeOversample = 5
    /// Line alpha at the front row.
    var ridgeAlpha = 0.85
    /// Line alpha at the back row.
    var ridgeBackAlpha = 0.0
    /// Shape of the alpha fade from the front row to the back: 1 is linear,
    /// below 1 holds the brightness further back, above 1 tails off
    /// gradually toward the back.
    var ridgeFadeCurve = 0.75
    /// Wall-clock interval between history rows, in ms.
    var ridgeRowMs = 33.0
    /// Width in points (a bell's sigma) each peak is spread into without
    /// losing height; 0 is identity. Applied before `ridgeSmooth`.
    var ridgeSpread = 1.0
    /// Neighbour blend applied to each point, 0...1; 0 is identity.
    var ridgeSmooth = 0.1
    /// Random texture on every resampled point as a fraction of its own
    /// height, re-rolled for every history row; 0 is off.
    var ridgeGrain = 0.03
    /// Fraction of the ridge width over which each end tapers to the baseline.
    var ridgeEdgeTaper = 0.22
    /// Easing factor per 60 Hz frame while a point rises.
    var ridgeAttack = 0.67
    /// Easing factor per 60 Hz frame while a point falls.
    var ridgeDecay = 0.5
    /// Level-curve exponent; 1 is identity.
    var ridgeGamma = 4.0
    /// Level-curve threshold, 0...1; 0 is identity.
    var ridgeFloorCut = 0.6
    /// Level-curve multiplier; 1 is identity.
    var ridgeGain = 1.15
    /// How far ahead of the audible position a paint reads its frame, in ms:
    /// what a paint takes to reach the screen, plus the half frame an onset
    /// waits for the next frame, plus the paints the attack easing needs to
    /// lift a point halfway (one, for the ridge). Each band's own filter lag
    /// is added on top by the frame source.
    var ridgeSyncLeadMs = 45.0
    /// Exponent on the frequency axis. A band's position along the log-spaced
    /// range (0 lowest, 1 highest) is drawn at that position raised to this
    /// power across the width: 1 is the plain log axis, below 1 gives the
    /// bass and low mids more of the width. 0.75 moves 100 Hz...1 kHz about a
    /// tenth of the width to the right, most around 300 Hz; the ends stay put.
    var ridgeAxisCurve = 0.75

    /// The shipped look.
    static let standard = RidgeParams()
}

