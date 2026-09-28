// Ported from ramusTV `RamusTV/Visualiser/Ridgeline.swift` (the row maths; the Core Graphics painter is not ported).

import CoreGraphics
import Foundation

// Ridgeline visualiser: a stack of horizontal lines rising from the bottom
// of the frame, each one a past moment of the spectrum drawn as a mountain
// profile, bass on the left and treble on the right, along a log frequency
// axis bent to give the low end more of the width (`axisSamples`).
//
// The front (lowest) row is the live spectrum; every `ridgeRowMs` the eased
// front row is copied into a history and the older rows step up one slot, so
// the stack scrolls upward and fades as it ages. A row is resampled from one
// point per band to several along a monotone cubic before it is textured and
// stored, so a peak is a run of near-equal points rather than one node with
// two lines meeting at it. Rows are painted back to front, and each row
// erases the layer under its own line before stroking it, so a near peak
// hides the rows behind it. The layer is transparent over the backdrop, so
// the erase reveals the backdrop rather than painting a background colour.
//
// Everything here is a pure function of its arguments, written with
// explicit output buffers so a paint allocates nothing per frame;
// `RidgePainter` owns the row buffers and the paint loop, and
// `RidgeGeometryBuilder` turns a painted frame into Metal geometry.
//
// Ported from ramus `ui/src/lib/ridgeline.ts`. Levels are stored as `Float`
// (the source's `Float32Array`) and combined in `Double` (the source's
// numbers), so results match the source to the last float bit.

/// JavaScript's `Math.round`: halves round toward positive infinity, so
/// `-2.5` rounds to `-2`.
@inline(__always)
func jsRound(_ x: Double) -> Double {
    (x + 0.5).rounded(.down)
}

/// Scratch buffers the row helpers reuse between calls, sized to the last
/// row seen, so a paint allocates nothing per frame. The source keeps these
/// as module-level buffers; here each painter owns one.
final class RidgeScratch {
    /// The bell for the last `sigma` and radius `spreadRow` ran with.
    var bell: [Float] = []
    var bellSigma = Double.nan
    /// Secant of each segment, for `monotoneTangents`.
    var secant: [Float] = []
    /// Slope at each point, for `resampleRow`.
    var slope: [Float] = []
    /// One row's line, and the same line closed along its baseline.
    var line: [CGPoint] = []
    var under: [CGPoint] = []
    /// Stroke colour of every row, rebuilt only when the row count, alphas
    /// or colour change, so a paint creates no colours.
    var rowColors: [CGColor] = []
    var rowColorsKey: RowColorKey?

    struct RowColorKey: Equatable {
        var rows: Int
        var alpha: Double
        var backAlpha: Double
        var fadeCurve: Double
        var color: RidgeRGB
    }

    init() {}
}

/// A stroke colour's channels, 0...1 in sRGB.
struct RidgeRGB: Equatable {
    var red: CGFloat
    var green: CGFloat
    var blue: CGFloat

    /// Ridge lines are white whatever the accent: the look is ink over the
    /// backdrop, and the backdrop already carries the accent (ramus
    /// `ui/src/components/FocusVisualizer.tsx`, `RIDGE_RGB`).
    static let white = RidgeRGB(red: 1, green: 1, blue: 1)
}

@inline(__always)
private func fill(_ out: inout [Float], _ value: Float) {
    for i in out.indices { out[i] = value }
}

@inline(__always)
private func copy(_ src: [Float], into out: inout [Float]) {
    for i in 0..<min(src.count, out.count) { out[i] = src[i] }
}

/// Average a stereo frame's bands into one 0...1 level per band, bass
/// first. `bands` holds `channels` runs of equal length; `out.count` must
/// equal that run length or the row is zeroed rather than read past its end.
///
/// ramus `ui/src/lib/ridgeline.ts`, `mixBandsInto`.
func mixBands(_ bands: [UInt8], channels: Int, into out: inout [Float]) {
    let n = out.count
    guard channels >= 1, bands.count == n * channels else {
        fill(&out, 0)
        return
    }
    let scale = 1 / (255 * Double(channels))
    for k in 0..<n {
        var sum = 0
        for c in 0..<channels { sum += Int(bands[c * n + k]) }
        out[k] = Float(Double(sum) * scale)
    }
}

/// Blend each point with its two neighbours: `amount` is the total weight
/// given to the neighbours (0 copies `src` unchanged, 1 replaces every point
/// with the mean of its neighbours). The ends reuse their own value for the
/// missing neighbour. Takes the single-band spikes off the line without
/// flattening a real peak.
///
/// ramus `ui/src/lib/ridgeline.ts`, `smoothRow`.
func smoothRow(_ src: [Float], into out: inout [Float], amount: Double) {
    let n = src.count
    let side = amount / 2
    let own = 1 - amount
    for i in 0..<n {
        let l = Double(src[i > 0 ? i - 1 : i])
        let r = Double(src[i < n - 1 ? i + 1 : i])
        out[i] = Float(own * Double(src[i]) + side * (l + r))
    }
}

/// Widen every peak without lowering it: each point becomes the largest of
/// itself and its neighbours scaled by a bell of width `sigma` (in points),
/// so a level lifts the points beside it to a falling fraction of itself.
/// Taking the maximum rather than the sum keeps a plateau a plateau and a
/// peak at its own height; only its flanks grow. A single band that clears
/// the level curve alone then reads as a peak instead of a one-point needle.
/// `sigma` 0 copies `src` unchanged.
///
/// ramus `ui/src/lib/ridgeline.ts`, `spreadRow`.
func spreadRow(
    _ src: [Float], into out: inout [Float], sigma: Double,
    scratch: RidgeScratch = RidgeScratch()
) {
    let n = src.count
    guard sigma > 0 else {
        copy(src, into: &out)
        return
    }
    guard n > 0 else { return }
    // Beyond three sigma the bell is under 1.2 % and changes nothing visible.
    let radius = min(n - 1, Int((sigma * 3).rounded(.up)))
    if sigma != scratch.bellSigma || scratch.bell.count != radius + 1 {
        scratch.bell = (0...radius).map { d in
            Float(exp(-Double(d * d) / (2 * sigma * sigma)))
        }
        scratch.bellSigma = sigma
    }
    let bell = scratch.bell
    for i in 0..<n {
        var best = Double(src[i])
        if radius >= 1 {
            for d in 1...radius {
                let k = Double(bell[d])
                if i - d >= 0 {
                    let v = Double(src[i - d]) * k
                    if v > best { best = v }
                }
                if i + d < n {
                    let v = Double(src[i + d]) * k
                    if v > best { best = v }
                }
            }
        }
        out[i] = Float(best)
    }
}

/// Texture a row: every point is scaled by `1 + amount * noise[i]`, with
/// `noise` in -1...1, and clamped at zero. Multiplicative, so a flat stretch
/// stays flat and only what rises gets grain in proportion to its height.
/// Meant for the resampled row, where the points are close enough that a
/// peak becomes a run of near-equal values rather than one node. `amount` 0
/// copies `src` unchanged.
///
/// ramus `ui/src/lib/ridgeline.ts`, `applyGrain`.
func applyGrain(_ src: [Float], into out: inout [Float], noise: [Float], amount: Double) {
    guard amount > 0 else {
        copy(src, into: &out)
        return
    }
    for i in 0..<src.count {
        let v = Double(src[i]) * (1 + amount * Double(noise[i]))
        out[i] = v > 0 ? Float(v) : 0
    }
}

/// Fill `noise` with fresh uniform values in -1..<1.
///
/// ramus `ui/src/lib/ridgeline.ts`, `rerollGrain`.
func rerollGrain<R: RandomNumberGenerator>(_ noise: inout [Float], using rng: inout R) {
    for i in noise.indices { noise[i] = Float.random(in: -1..<1, using: &rng) }
}

/// The grain's random source: SplitMix64, cheap and seedable so a render can
/// be repeated exactly.
struct RidgeRandom: RandomNumberGenerator {
    private var state: UInt64

    init(seed: UInt64) {
        state = seed
    }

    init() {
        state = UInt64.random(in: .min ... .max)
    }

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
}

/// Per-point multiplier that brings both ends of a row down to its baseline:
/// a raised-cosine ramp over the first and last `taper` fraction of the
/// points (rounded to whole points) and 1 in between. `taper` 0 returns a
/// flat window. Without it the lowest and highest bands would end the line
/// mid-air.
///
/// ramus `ui/src/lib/ridgeline.ts`, `edgeWindow`.
func edgeWindow(count n: Int, taper: Double) -> [Float] {
    guard n > 0 else { return [] }
    var w = [Float](repeating: 1, count: n)
    let ramp = Int(jsRound((taper > 0 ? taper : 0) * Double(n)))
    for i in 0..<n {
        let fromEdge = min(i, n - 1 - i)
        if ramp <= 0 || fromEdge >= ramp {
            w[i] = 1
        } else {
            w[i] = Float(0.5 - 0.5 * cos(Double.pi * Double(fromEdge) / Double(ramp)))
        }
    }
    return w
}

/// Fixed-size ring of past rows, newest first. Every slot is allocated up
/// front and overwritten in place, so pushing and reading allocate nothing
/// per frame.
///
/// ramus `ui/src/lib/ridgeline.ts`, `RidgeHistory`.
final class RidgeHistory {
    let rows: Int
    let width: Int
    private var slots: [[Float]]
    /// A row of silence, handed out for any slot the ring doesn't have.
    private let silence: [Float]
    /// Slot the next push writes; the newest row is the slot before it.
    private var head = 0

    init(rows: Int, width: Int) {
        self.rows = max(1, rows)
        self.width = max(0, width)
        let w = self.width
        slots = (0..<self.rows).map { _ in [Float](repeating: 0, count: w) }
        silence = [Float](repeating: 0, count: w)
    }

    /// Copy `row` in as the newest; the oldest row is dropped. A row wider
    /// than the ring is cut to fit, a narrower one is padded with silence.
    func push(_ row: [Float]) {
        let width = self.width
        let m = min(row.count, width)
        slots[head].withUnsafeMutableBufferPointer { dst in
            for i in 0..<m { dst[i] = row[i] }
            for i in m..<width { dst[i] = 0 }
        }
        head = (head + 1) % rows
    }

    /// Row `k` back from the newest (0 = newest). Silence until pushed, and
    /// silence for any `k` the ring doesn't hold.
    func get(_ k: Int) -> [Float] {
        guard k >= 0, k < rows else { return silence }
        let i = ((head - 1 - k) % rows + rows) % rows
        return slots[i]
    }

    /// Every row back to silence.
    func clear() {
        for s in slots.indices {
            slots[s].withUnsafeMutableBufferPointer { $0.update(repeating: 0) }
        }
        head = 0
    }
}

/// Where one row of the stack sits.
///
/// ramus `ui/src/lib/ridgeline.ts`, `RidgeRowGeometry`.
struct RidgeRowGeometry: Equatable {
    /// Y of the row's resting line, in points from the top.
    var baseline: Double
    /// Points a full-scale point rises above the baseline.
    var scale: Double
    /// Line alpha.
    var alpha: Double
}

/// Where row `k` of `rows` sits in a frame `h` points tall: the front row's
/// baseline is `ridgeBottom` up from the bottom edge, the back row's is
/// `ridgeHeight` above that, and rows are spaced evenly between. Peak scale
/// eases linearly from the front value to the back one, and alpha along
/// `(1 - depth) ^ ridgeFadeCurve`.
///
/// ramus `ui/src/lib/ridgeline.ts`, `ridgeRow`.
func ridgeRow(_ k: Int, rows: Int, height h: Double, params p: RidgeParams) -> RidgeRowGeometry {
    ridgeRow(depth: rows > 1 ? Double(k) / Double(rows - 1) : 0, height: h, params: p)
}

/// `ridgeRow` at a fractional `depth` (0 front, 1 back), for a stack that
/// moves between row slots.
func ridgeRow(depth: Double, height h: Double, params p: RidgeParams) -> RidgeRowGeometry {
    let front = h * (1 - p.ridgeBottom)
    return RidgeRowGeometry(
        baseline: front - depth * h * p.ridgeHeight,
        scale: h * p.ridgePeak * (1 + (p.ridgeDepthScale - 1) * depth),
        alpha: p.ridgeBackAlpha
            + (p.ridgeAlpha - p.ridgeBackAlpha) * pow(max(0, 1 - depth), p.ridgeFadeCurve))
}

/// Slope at every point for a curve through `y` that never overshoots
/// between neighbouring points (Fritsch–Carlson monotone cubic
/// interpolation). Interior slopes start as the mean of the two secants and
/// are set to zero at every local extremum and at both ends of a flat
/// segment, then limited so each segment stays monotone. A peak therefore
/// gets a level tangent at its top and a rounded foot below it instead of two
/// straight lines meeting at a corner. `y` and `out` are the same length;
/// slopes are in y units per point.
///
/// ramus `ui/src/lib/ridgeline.ts`, `monotoneTangents`.
func monotoneTangents(
    _ y: [Float], into out: inout [Float], scratch: RidgeScratch = RidgeScratch()
) {
    let n = y.count
    if n == 0 { return }
    if n == 1 {
        out[0] = 0
        return
    }
    if scratch.secant.count != n - 1 {
        scratch.secant = [Float](repeating: 0, count: n - 1)
    }
    scratch.secant.withUnsafeMutableBufferPointer { d in
        // Secant of each segment, then the first-guess slope at each point.
        for k in 0..<(n - 1) { d[k] = Float(Double(y[k + 1]) - Double(y[k])) }
        out[0] = d[0]
        out[n - 1] = d[n - 2]
        for k in 1..<(n - 1) {
            let l = Double(d[k - 1]), r = Double(d[k])
            out[k] = l * r <= 0 ? 0 : Float((l + r) / 2)
        }
        // Keep every segment monotone: a flat segment gets flat ends, and a
        // segment whose end slopes are too steep for its own secant has them
        // scaled back onto the circle of radius 3.
        for k in 0..<(n - 1) {
            if d[k] == 0 {
                out[k] = 0
                out[k + 1] = 0
                continue
            }
            let dk = Double(d[k])
            let a = Double(out[k]) / dk
            let b = Double(out[k + 1]) / dk
            let r2 = a * a + b * b
            if r2 > 9 {
                let t = 3 / r2.squareRoot()
                out[k] = Float(t * a * dk)
                out[k + 1] = Float(t * b * dk)
            }
        }
    }
}

/// Where each point of a resampled row falls on the band row it is read
/// from: `out[j]` is a position between 0 (the lowest band) and `bands - 1`
/// (the highest) for point `j` of `out.count`, the points spread evenly
/// across the width. The bands are log-spaced in frequency, and `curve`
/// bends the axis: a band's position along the range, 0...1, is drawn at
/// that position raised to `curve`, so 1 spreads the bands evenly and below
/// 1 widens the low end and narrows the top. The ends stay put.
///
/// ramus `ui/src/lib/ridgeline.ts`, `axisSamples`.
func axisSamples(bands: Int, curve: Double, into out: inout [Float]) {
    let last = out.count - 1
    guard last >= 0 else { return }
    let inverse = curve > 0 ? 1 / curve : 1
    let span = Double(max(0, bands - 1))
    for j in 0...last {
        let x = last > 0 ? Double(j) / Double(last) : 0
        out[j] = Float(pow(x, inverse) * span)
    }
}

/// Resample a row along the monotone cubic through its points
/// (`monotoneTangents`), reading it at the band positions in `at` (from
/// `axisSamples`), so the curve's shape survives as plain points. `out` must
/// be as long as `at` or it is zeroed.
///
/// ramus `ui/src/lib/ridgeline.ts`, `resampleRowAt`.
func resampleRow(
    _ src: [Float], at: [Float], into out: inout [Float],
    scratch: RidgeScratch = RidgeScratch()
) {
    let n = src.count
    if n == 0 || out.count != at.count {
        fill(&out, 0)
        return
    }
    if n == 1 {
        fill(&out, src[0])
        return
    }
    if scratch.slope.count != n {
        scratch.slope = [Float](repeating: 0, count: n)
    }
    monotoneTangents(src, into: &scratch.slope, scratch: scratch)
    let m = scratch.slope
    let top = Double(n - 1)
    for j in 0..<out.count {
        let c = min(top, max(0, Double(at[j])))
        let i = min(n - 2, Int(c.rounded(.down)))
        let t = c - Double(i)
        let t2 = t * t
        let t3 = t2 * t
        let v = (2 * t3 - 3 * t2 + 1) * Double(src[i])
            + (t3 - 2 * t2 + t) * Double(m[i])
            + (-2 * t3 + 3 * t2) * Double(src[i + 1])
            + (t3 - t2) * Double(m[i + 1])
        out[j] = Float(v)
    }
}

