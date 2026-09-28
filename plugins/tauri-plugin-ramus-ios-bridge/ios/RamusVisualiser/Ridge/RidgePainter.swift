// Ported from ramusTV `RamusTV/Visualiser/RidgePainter.swift` (Metal path only).

import Foundation

/// The ridge's per-frame state: reads the frame source, shapes and eases the
/// levels, cuts history rows and decides whether the picture changed.
///
/// Ported from the ridge half of ramus
/// `ui/src/components/FocusVisualizer.tsx` (`CanvasLayer`, `render`). Each
/// display frame the host calls `step(now:frameTime:source:height:)`; when
/// it returns true the host reads `frame()` and draws it. Reading changes
/// nothing, so the host may also draw again on its own (after a resize,
/// say) to repaint the same state.
///
/// Sync: the paint reads the frame for the moment it reaches the screen,
/// `ridgeSyncLeadMs + leadOffsetMs` after `now`; the source clocks that off
/// the audible position and adds each band's filter lag. No frame (paused,
/// stopped, stalled, between bursts) decays the line to silence rather than
/// freezing it.
///
/// Shaping: the two channels are averaged into one point per band, bass
/// first; each point runs through the level curve, then every peak is
/// widened (`spreadRow`) and blended with its neighbours (`smoothRow`), and
/// the result is spring-eased with separate attack and decay factors
/// normalised to a 60 Hz reference, so the motion is the same at any refresh
/// rate. The eased row is resampled onto `ridgeOversample` points per band
/// along the bent frequency axis, textured with grain, and every
/// `ridgeRowMs` of wall clock copied into the history.
///
/// Idle: once the live line has been flat for longer than the stack takes to
/// carry a row off the top, every row is a flat rule and `step` returns
/// false until something moves again, so a pause costs no paints. While
/// settled no rows are cut.
///
/// Buffers are sized on the first frame and resized if the band count ever
/// changes; nothing is allocated per frame otherwise.
final class RidgePainter {
    /// Bands per channel assumed until the first frame arrives.
    static let defaultBandCount = 64
    /// Channels per frame (left, right).
    static let channels = 2
    /// A point that rises less than this many points on the tallest row
    /// counts as flat.
    static let minVisibleHeight = 0.5
    /// The easing factors are tuned against a 60 Hz frame.
    static let easeReferenceDtMs = 1000.0 / 60
    /// Longest frame delta fed to the easing, so a long hitch (backgrounded,
    /// debugger stop) doesn't push `pow` to extremes; about six reference
    /// frames already eases all the way.
    static let easeDtClampMs = 100.0

    var params: RidgeParams
    /// A deeper stack for a full-screen overlay: `ridgeFullScreenRows` rows at
    /// the usual spacing (ramus `FocusVisualizer`'s `fullScreen` prop).
    var fullScreen = false
    /// Added to `ridgeSyncLeadMs` when reading the source, in ms; corrects
    /// for output latency the audible clock doesn't see.
    var leadOffsetMs = 0.0
    /// Stroke colour of every row.
    var color: RidgeRGB = .white
    /// Refresh period of the display the frames are shown on, in ms. When
    /// set, the row interval is a whole number of these (`rowPeriodMs`).
    var framePeriodMs: Double?

    /// The eased level of each band, 0...1, bass first.
    private(set) var levels: [Float]
    /// Past rows, newest first; nil until the first paint with two or more bands.
    private(set) var history: RidgeHistory?
    /// Rows cut into the history since the painter was made.
    private(set) var rowsCut = 0

    // Per-band buffers: the target frame, the target after the level curve,
    // and the widened and neighbour-blended copies of it.
    private var target: [Float]
    private var shaped: [Float]
    private var spread: [Float]
    private var smoothed: [Float]

    // All on the fine grid the eased row is resampled to: the resampled row,
    // where each fine point reads the band row (for the axis curve and band
    // count it was built with), the per-point edge window (for the taper it
    // was built with), and the grain: one noise value per fine point, applied
    // to the live line as drawn and frozen into each history row when it is
    // cut, then re-rolled, so every row carries its own texture.
    private var fine: [Float] = []
    private var axis: [Float] = []
    private var axisBands = 0
    private var axisCurve = Double.nan
    private var taper: [Float] = []
    private var taperAmount = Double.nan
    private var grain: [Float] = []
    private var grained: [Float] = []
    private var random: RidgeRandom
    private let scratch = RidgeScratch()

    /// Time of the previous step, for the easing's frame delta.
    private var lastStepAt: Double?
    /// When the last history row was cut.
    private var lastRowAt = 0.0
    /// Since when the live line has been flat; nil while it isn't.
    private var quietSince: Double?
    /// Row count of the last paint.
    private var paintedRows = 0
    /// The layer was wiped (resized); the next step repaints whatever it holds.
    private var wiped = false
    /// Rows and layout the last paint was prepared with; `drawRows` is 0 when
    /// there was nothing to draw.
    private var drawRows = 0
    private var drawLayout: RidgeParams
    /// Glide phase of the last step: whole display frames since the latest
    /// cut, and frames per row (`RidgeFrame`).
    private var framesSinceCut = 0
    private var framesPerRow = 1
    /// `frameTime` of the step that cut the latest row, when it had one.
    private var cutFrameTime: Double?

    init(params: RidgeParams = .standard, random: RidgeRandom = RidgeRandom()) {
        self.params = params
        self.random = random
        drawLayout = params
        let n = Self.defaultBandCount
        levels = [Float](repeating: 0, count: n)
        target = [Float](repeating: 0, count: n)
        shaped = [Float](repeating: 0, count: n)
        spread = [Float](repeating: 0, count: n)
        smoothed = [Float](repeating: 0, count: n)
    }

    /// Rows in the stack, the live front row included.
    var rowCount: Int {
        max(1, fullScreen ? params.ridgeFullScreenRows : params.ridgeRows)
    }

    /// The layout the rows are drawn with: the parameters, with a full-screen
    /// stack's height stretched so its extra rows sit at the usual spacing.
    var layout: RidgeParams {
        var p = params
        if fullScreen && params.ridgeRows > 1 {
            p.ridgeHeight = params.ridgeHeight * Double(rowCount - 1) / Double(params.ridgeRows - 1)
        }
        return p
    }

    /// The layer was wiped or resized: the next step repaints even if the
    /// picture is settled.
    func invalidate() {
        wiped = true
    }

    /// Back to silence: eased levels, history and timers.
    func reset() {
        for i in levels.indices { levels[i] = 0 }
        history?.clear()
        lastStepAt = nil
        lastRowAt = 0
        quietSince = nil
        wiped = true
    }

    /// The level curve: zero at or below `ridgeFloorCut` with the rest
    /// rescaled to full range, raised to `ridgeGamma`, times `ridgeGain`,
    /// clamped to 1.
    static func shape(_ level: Float, params p: RidgeParams) -> Float {
        guard p.ridgeFloorCut > 0 || p.ridgeGamma != 1 || p.ridgeGain != 1 else { return level }
        var v = Double(level)
        v = v <= p.ridgeFloorCut ? 0 : (v - p.ridgeFloorCut) / (1 - p.ridgeFloorCut)
        if p.ridgeGamma != 1 { v = pow(v, p.ridgeGamma) }
        return Float(min(1, v * p.ridgeGain))
    }

    /// Share of the remaining distance a point moves in a frame `dtMs` long
    /// for an easing factor tuned per 60 Hz frame:
    /// `1 - (1 - ease) ^ (dt / referenceDt)`, with `dt` clamped.
    static func easeAlpha(_ ease: Double, dtMs: Double) -> Double {
        let dt = min(dtMs, easeDtClampMs)
        guard dt > 0 else { return 0 }
        return 1 - pow(1 - ease, dt / easeReferenceDtMs)
    }

    /// The interval between history rows: `rowMs`, rounded to a whole
    /// number of display frames (at least one) when the frame period is
    /// known, so every row stays up for the same number of frames. Left
    /// unrounded, 33 ms rows on a 60 Hz display (two frames are 33.4 ms)
    /// bank a little credit every row and spend it as a row one frame after
    /// the last, about every 1.5 s, and the whole stack jumps twice in a
    /// row.
    static func rowPeriodMs(rowMs: Double, framePeriodMs: Double?) -> Double {
        guard let frame = framePeriodMs, frame > 0 else { return rowMs }
        return max(1, (rowMs / frame).rounded()) * frame
    }

    /// The new cut time when a history row is due at `now`, else nil. The
    /// cut time steps by the period so the cadence keeps its fractional
    /// credit rather than rounding up to the frame rate; after a hitch longer
    /// than two periods it resyncs instead of replaying the gap as a burst.
    /// A row counts as due `toleranceMs` early, so a frame callback that runs
    /// a little ahead of its slot still cuts the row that frame is due.
    static func nextRowAt(now: Double, lastRowAt: Double, periodMs: Double, toleranceMs: Double = 0) -> Double? {
        guard now - lastRowAt >= periodMs - toleranceMs else { return nil }
        return now - lastRowAt > 2 * periodMs ? now : lastRowAt + periodMs
    }

    /// Whether a live line whose tallest point is `peak` would rise under
    /// half a point on the tallest row of a frame `height` points tall.
    func isFlat(peak: Float, height: Double) -> Bool {
        let rise = height * params.ridgePeak * max(1, params.ridgeDepthScale)
        return Double(peak) * rise < Self.minVisibleHeight
    }

    /// Advance one display frame. `now` is milliseconds on the
    /// `CACurrentMediaTime()` clock and `height` the frame height in points.
    /// `frameTime`, on the same clock, is the display frame the step draws
    /// for (a display link's vsync-aligned timestamp); with it, the glide
    /// phase counts display frames, so a late callback can't move it.
    /// Returns true when the layer must be cleared and repainted with `draw`.
    func step(now: Double, frameTime: Double? = nil, source: RidgeFrameSource?, height: Double) -> Bool {
        let repaintForced = wiped
        wiped = false
        let rawDelta = lastStepAt.map { now - $0 } ?? 0
        lastStepAt = now
        let p = params

        var haveFrame = false
        if let source, source.isPlaying {
            let lead = (p.ridgeSyncLeadMs + leadOffsetMs) / 1000
            if let bands = source.bands(now: now, leadSec: lead) {
                let points = bands.count / Self.channels
                if points != levels.count { resize(points) }
                mixBands(bands, channels: Self.channels, into: &target)
                haveFrame = true
            }
        }
        if !haveFrame {
            for i in target.indices { target[i] = 0 }
        }

        let pointCount = levels.count
        for i in 0..<pointCount { shaped[i] = Self.shape(target[i], params: p) }
        // Widen each peak, then blend each point with its neighbours, both
        // after the curve: a floor cut leaves one band standing alone as a
        // needle, and the spread turns it back into a peak while the blend
        // softens the hard zeros around it.
        var useSpread = false
        var useSmoothed = false
        if p.ridgeSpread > 0 {
            spreadRow(shaped, into: &spread, sigma: p.ridgeSpread, scratch: scratch)
            useSpread = true
        }
        if p.ridgeSmooth > 0 {
            smoothRow(useSpread ? spread : shaped, into: &smoothed, amount: p.ridgeSmooth)
            useSmoothed = true
        }
        let alphaAttack = Self.easeAlpha(p.ridgeAttack, dtMs: rawDelta)
        let alphaDecay = Self.easeAlpha(p.ridgeDecay, dtMs: rawDelta)
        var peak: Float = 0
        for i in 0..<pointCount {
            let prev = Double(levels[i])
            let level = Double(useSmoothed ? smoothed[i] : useSpread ? spread[i] : shaped[i])
            let alpha = level > prev ? alphaAttack : alphaDecay
            let eased = Float(prev + (level - prev) * alpha)
            levels[i] = eased
            if eased > peak { peak = eased }
        }

        // Flat for longer than the stack takes to carry a row off the top
        // means every history row is flat too, so the picture is a fixed set
        // of rules and repainting it is wasted work. A wiped layer or a
        // changed row count still gets one paint.
        let rows = rowCount
        if !isFlat(peak: peak, height: height) {
            quietSince = nil
        } else if quietSince == nil {
            quietSince = now
        }
        let settled = quietSince.map { now - $0 > Double(rows + 1) * p.ridgeRowMs } ?? false
        guard !settled || repaintForced || rows != paintedRows else { return false }
        paintedRows = rows
        drawRows = 0

        // One point has no interval to resample; the row count and the fine
        // grid both come out of the intervals between points.
        guard pointCount >= 2 else { return true }
        let perBand = min(8, max(1, p.ridgeOversample))
        let fineCount = (pointCount - 1) * perBand + 1
        if fine.count != fineCount {
            fine = [Float](repeating: 0, count: fineCount)
            grain = [Float](repeating: 0, count: fineCount)
            grained = [Float](repeating: 0, count: fineCount)
            rerollGrain(&grain, using: &random)
        }
        // One row more than the stack shows behind the live row, so a
        // renderer that moves the stack between cuts has a row to carry past
        // the back while it fades out.
        let historyRows = rows
        if history?.rows != historyRows || history?.width != fineCount {
            history = RidgeHistory(rows: historyRows, width: fineCount)
        }
        if taper.count != fineCount || taperAmount != p.ridgeEdgeTaper {
            taper = edgeWindow(count: fineCount, taper: p.ridgeEdgeTaper)
            taperAmount = p.ridgeEdgeTaper
        }
        if axis.count != fineCount || axisBands != pointCount || axisCurve != p.ridgeAxisCurve {
            if axis.count != fineCount { axis = [Float](repeating: 0, count: fineCount) }
            axisSamples(bands: pointCount, curve: p.ridgeAxisCurve, into: &axis)
            axisBands = pointCount
            axisCurve = p.ridgeAxisCurve
        }
        resampleRow(levels, at: axis, into: &fine, scratch: scratch)
        // A row is cut every `ridgeRowMs` of wall clock (rounded to whole
        // display frames when the refresh rate is known), so the stack
        // scrolls at one speed whatever the refresh rate. It keeps scrolling through
        // a pause, carrying the flat line up until every row is flat; a flat
        // row is a rule at its baseline.
        let rowPeriod = Self.rowPeriodMs(rowMs: p.ridgeRowMs, framePeriodMs: framePeriodMs)
        var cut = false
        if let next = Self.nextRowAt(
            now: now, lastRowAt: lastRowAt, periodMs: rowPeriod, toleranceMs: (framePeriodMs ?? 0) / 2) {
            applyGrain(fine, into: &grained, noise: grain, amount: p.ridgeGrain)
            history?.push(grained)
            rowsCut += 1
            rerollGrain(&grain, using: &random)
            lastRowAt = next
            cutFrameTime = frameTime
            cut = true
        }
        if let frame = framePeriodMs, frame > 0 {
            framesPerRow = max(1, Int((p.ridgeRowMs / frame).rounded()))
            // Display frames since the cut, from vsync times when there are
            // any; the step that cuts is frame 0 whenever it runs.
            let elapsed: Double
            if let frameTime, let cutFrameTime {
                elapsed = frameTime - cutFrameTime
            } else {
                elapsed = now - lastRowAt
            }
            framesSinceCut = cut ? 0 : min(framesPerRow - 1, max(0, Int((elapsed / frame).rounded())))
        } else {
            framesPerRow = 1
            framesSinceCut = 0
        }
        applyGrain(fine, into: &grained, noise: grain, amount: p.ridgeGrain)
        drawRows = rows
        drawLayout = layout
        return true
    }

    /// The state the last `step` prepared, for a renderer that builds its
    /// own geometry; nil before the first paint or with fewer than two bands.
    func frame() -> RidgeFrame? {
        guard drawRows > 0, let history else { return nil }
        let live = grained
        return RidgeFrame(
            rows: drawRows, levels: { k in k == 0 ? live : history.get(k - 1) },
            historyRows: history.rows, edge: taper, color: color, layout: drawLayout,
            framesSinceCut: framesSinceCut, framesPerRow: framesPerRow)
    }

    /// Resize the per-band buffers; the eased levels restart from silence.
    private func resize(_ points: Int) {
        levels = [Float](repeating: 0, count: points)
        target = [Float](repeating: 0, count: points)
        shaped = [Float](repeating: 0, count: points)
        spread = [Float](repeating: 0, count: points)
        smoothed = [Float](repeating: 0, count: points)
    }
}
