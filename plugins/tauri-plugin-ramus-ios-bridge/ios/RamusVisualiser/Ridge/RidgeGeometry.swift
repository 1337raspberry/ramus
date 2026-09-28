// Ported from ramusTV `RamusTV/Visualiser/RidgeGeometry.swift`.

import Foundation

/// One trapezoid of a row's erase, in device pixels: from `x0` to `x1`,
/// between a top edge lying on the row's line and the row's floor. The top
/// edge is antialiased against the full line segment `a`–`b`.
///
/// The layout matches `EraseSpan` in `ridgeShaderSource` (`Shaders/RidgeShaderSource.swift`).
struct RidgeEraseSpan: Equatable {
    var a: SIMD2<Float>
    var b: SIMD2<Float>
    var x0: Float
    var x1: Float
    /// Top edge y at `x0` and `x1`.
    var top0: Float
    var top1: Float
    var floor: Float
}

/// One segment of a row's line, `p1`–`p2`, with the points either side of
/// it (`p0`, `p3`; repeated at the row's ends) so the stroke shader can join
/// neighbouring segments without drawing a pixel twice.
///
/// The layout matches `StrokeSegment` in `ridgeShaderSource` (`Shaders/RidgeShaderSource.swift`).
struct RidgeStrokeSegment: Equatable {
    var p0: SIMD2<Float>
    var p1: SIMD2<Float>
    var p2: SIMD2<Float>
    var p3: SIMD2<Float>
    var alpha: Float
    var flags: UInt32

    /// `p1` is the row's first point: butt-capped, no segment before it.
    static let openStart: UInt32 = 1
    /// `p2` is the row's last point: butt-capped, no segment after it.
    static let openEnd: UInt32 = 2
}

/// The draw ranges of one row: its erase, then its stroke.
struct RidgeRowDraw: Equatable {
    var spans: Range<Int>
    var segments: Range<Int>
}

/// Everything one frame of the ridge draws, rows back to front. Reused from
/// frame to frame, so building into it allocates nothing once its buffers
/// have grown to the stack's size.
struct RidgeDrawList {
    var spans: [RidgeEraseSpan] = []
    var segments: [RidgeStrokeSegment] = []
    var rows: [RidgeRowDraw] = []
    /// Stroke width in device pixels.
    var lineWidth: Float = 0
    var color: RidgeRGB = .white

    mutating func removeAll() {
        spans.removeAll(keepingCapacity: true)
        segments.removeAll(keepingCapacity: true)
        rows.removeAll(keepingCapacity: true)
    }
}

/// Turns a `RidgeFrame` into a `RidgeDrawList` for a target, with the row
/// placement of `RidgeStackLayout` and the point maths of `drawRidgeline`:
/// the points span `ridgeSpan` of the width, centred, each lifted by its
/// level times its edge window times the row's rise. Each row erases where
/// its line rises above its floor, meeting the floor exactly where the line
/// crosses it, and strokes every segment unless its alpha is 0.
final class RidgeGeometryBuilder {
    private var placements: [RidgeRowPlacement] = []
    /// One row at a time: its points (and their y alone, for the floor
    /// tests), then its spans and segments, which go into the list with one
    /// bulk copy per row rather than one append per item. The loops write
    /// through buffer pointers, which keeps them cheap in unoptimised builds
    /// too.
    private var points: [SIMD2<Float>] = []
    private var ys: [Float] = []
    private var rowSpans: [RidgeEraseSpan] = []
    private var rowSegments: [RidgeStrokeSegment] = []

    init() {}

    func build(_ frame: RidgeFrame?, target: RidgeTarget, glide: Bool, into list: inout RidgeDrawList) {
        list.removeAll()
        guard let frame else { return }
        let n = frame.edge.count
        guard n >= 2, frame.rows >= 1 else { return }
        RidgeStackLayout.place(frame, target: target, glide: glide, into: &placements)
        guard !placements.isEmpty else { return }
        let p = frame.layout
        list.lineWidth = Float(p.ridgeLineWidth * target.scale)
        list.color = frame.color
        let fieldW = Double(target.pixelWidth) * p.ridgeSpan
        let fieldX = (Double(target.pixelWidth) - fieldW) / 2
        let step = fieldW / Double(n - 1)
        if points.count != n {
            points = [SIMD2<Float>](repeating: .zero, count: n)
            ys = [Float](repeating: 0, count: n)
            rowSpans = [RidgeEraseSpan](
                repeating: RidgeEraseSpan(a: .zero, b: .zero, x0: 0, x1: 0, top0: 0, top1: 0, floor: 0),
                count: n - 1)
            rowSegments = [RidgeStrokeSegment](
                repeating: RidgeStrokeSegment(p0: .zero, p1: .zero, p2: .zero, p3: .zero, alpha: 0, flags: 0),
                count: n - 1)
        }
        let most = placements.count * (n - 1)
        list.spans.reserveCapacity(most)
        list.segments.reserveCapacity(most)
        list.rows.reserveCapacity(placements.count)

        for place in placements {
            let values = frame.levels(place.row)
            let count = min(values.count, n)
            let baseline = place.baseline, rise = place.rise
            values.withUnsafeBufferPointer { vb in
                frame.edge.withUnsafeBufferPointer { eb in
                    ys.withUnsafeMutableBufferPointer { yb in
                        points.withUnsafeMutableBufferPointer { pb in
                            guard let e = eb.baseAddress, let y = yb.baseAddress, let pts = pb.baseAddress
                            else { return }
                            let v = vb.baseAddress
                            for i in 0..<n {
                                let level = i < count ? Double(v![i]) : 0
                                let py = Float(baseline - level * Double(e[i]) * rise)
                                y[i] = py
                                pts[i] = SIMD2(Float(fieldX + Double(i) * step), py)
                            }
                        }
                    }
                }
            }
            let spanStart = list.spans.count
            if let floor = place.floor {
                let k = fillErase(floor: floor)
                list.spans.append(contentsOf: rowSpans[0..<k])
            }
            let segmentStart = list.segments.count
            if place.alpha > 0 {
                fillStroke(alpha: Float(place.alpha))
                list.segments.append(contentsOf: rowSegments)
            }
            list.rows.append(RidgeRowDraw(
                spans: spanStart..<list.spans.count, segments: segmentStart..<list.segments.count))
        }
    }

    /// Writes the row's erase spans to the front of `rowSpans`; returns how
    /// many.
    private func fillErase(floor: Double) -> Int {
        let f = Float(floor)
        var k = 0
        points.withUnsafeBufferPointer { pb in
            ys.withUnsafeBufferPointer { yb in
                rowSpans.withUnsafeMutableBufferPointer { ob in
                    guard let pts = pb.baseAddress, let y = yb.baseAddress, let out = ob.baseAddress else { return }
                    for i in 0..<(pb.count - 1) {
                        let ay = Double(y[i]), by = Double(y[i + 1])
                        let aAbove = ay < floor, bAbove = by < floor
                        guard aAbove || bAbove else { continue }
                        let a = pts[i], b = pts[i + 1]
                        var span = RidgeEraseSpan(
                            a: a, b: b, x0: a.x, x1: b.x, top0: y[i], top1: y[i + 1], floor: f)
                        if aAbove != bAbove {
                            let ax = Double(a.x), bx = Double(b.x)
                            let t = (floor - ay) / (by - ay)
                            let xc = Float(ax + t * (bx - ax))
                            if aAbove {
                                span.x1 = xc
                                span.top1 = f
                            } else {
                                span.x0 = xc
                                span.top0 = f
                            }
                        }
                        out[k] = span
                        k += 1
                    }
                }
            }
        }
        return k
    }

    /// Writes the row's stroke segments over `rowSegments`, sliding a
    /// window of four points along the row.
    private func fillStroke(alpha: Float) {
        points.withUnsafeBufferPointer { buffer in
            rowSegments.withUnsafeMutableBufferPointer { segments in
                guard let pts = buffer.baseAddress, let out = segments.baseAddress else { return }
                let last = buffer.count - 1
                var p0 = pts[0], p1 = pts[0], p2 = pts[1]
                for i in 0..<last {
                    let p3 = i + 2 > last ? p2 : pts[i + 2]
                    var flags: UInt32 = 0
                    if i == 0 { flags |= RidgeStrokeSegment.openStart }
                    if i == last - 1 { flags |= RidgeStrokeSegment.openEnd }
                    out[i] = RidgeStrokeSegment(p0: p0, p1: p1, p2: p2, p3: p3, alpha: alpha, flags: flags)
                    p0 = p1
                    p1 = p2
                    p2 = p3
                }
            }
        }
    }
}
