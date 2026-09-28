// Ported from ramusTV `RamusTV/Visualiser/RidgeStackLayout.swift`.

import Foundation

/// A render target in whole device pixels.
struct RidgeTarget: Equatable {
    var pixelWidth: Int
    var pixelHeight: Int
    /// Device pixels per point.
    var scale: Double

    /// Width in points.
    var width: Double { Double(pixelWidth) / scale }
    /// Height in points.
    var height: Double { Double(pixelHeight) / scale }
}

/// Where one drawn row of the stack sits, in device pixels from the top
/// left.
struct RidgeRowPlacement: Equatable {
    /// Stack row, numbered as `RidgeFrame.levels` numbers them.
    var row: Int
    /// Y of the row's resting line.
    var baseline: Double
    /// Pixels a full-scale point rises above the baseline.
    var rise: Double
    /// Line alpha.
    var alpha: Double
    /// Lowest y the row's erase reaches; nil for the farthest drawn row,
    /// which has nothing behind it to hide.
    var floor: Double?
}

/// Places the rows of a `RidgeFrame` on a target's pixel grid.
///
/// The grid rule is `drawRidgeline`'s: the front row's stroke has its upper
/// edge on a pixel boundary and rows sit a whole number of pixels apart, so
/// every flat rule covers the same pixels.
///
/// With `glide`, the history stack moves on every display frame rather than
/// once per row: `framesSinceCut / framesPerRow` of the way from one slot to
/// the next, rounded to whole pixels and the same for every history row, so
/// the spacing stays even and on the grid. The newest history row starts on
/// the live row and is not drawn while it is within a pixel of it; the
/// oldest travels past the back slot, where its alpha is 0. Rise and alpha
/// follow each row's fractional depth, so nothing pops at a cut.
///
/// Each row's erase stops a pixel below the stroke of the next drawn row
/// behind it (or at its own baseline if that is higher): on a cleared layer
/// nothing behind reaches further down.
enum RidgeStackLayout {
    /// Placements for every row to draw, back to front, into `out`.
    static func place(_ frame: RidgeFrame, target: RidgeTarget, glide: Bool, into out: inout [RidgeRowPlacement]) {
        out.removeAll(keepingCapacity: true)
        let rows = frame.rows
        guard rows >= 1, target.pixelWidth > 0, target.pixelHeight > 0, target.scale > 0 else { return }
        let p = frame.layout
        let scale = target.scale
        let h = target.height
        let halfStroke = p.ridgeLineWidth * scale / 2
        let frontY = ridgeRow(0, rows: rows, height: h, params: p).baseline
        let backY = ridgeRow(rows - 1, rows: rows, height: h, params: p).baseline
        let front = jsRound(frontY * scale - halfStroke) + halfStroke
        let pitch = rows > 1 ? jsRound((frontY - backY) / Double(rows - 1) * scale) : 0

        func append(row: Int, depth: Double, baseline: Double) {
            let g = ridgeRow(depth: depth, height: h, params: p)
            out.append(RidgeRowPlacement(row: row, baseline: baseline, rise: g.scale * scale, alpha: g.alpha, floor: nil))
        }

        // Front to back first.
        append(row: 0, depth: 0, baseline: front)
        if rows > 1 {
            let span = Double(rows - 1)
            let n = max(1, frame.framesPerRow)
            let f = glide ? Double(min(max(frame.framesSinceCut, 0), n - 1)) / Double(n) : 0
            let shift = glide ? jsRound(f * pitch) : 0
            for j in 0..<max(0, frame.historyRows) {
                let slot = glide ? Double(j) + f : Double(j + 1)
                let depth = slot / span
                if depth > 1 { break }
                if glide && j == 0 && shift < 1 { continue }
                let baseline = glide ? front - Double(j) * pitch - shift : front - Double(j + 1) * pitch
                append(row: j + 1, depth: depth, baseline: baseline)
            }
        }
        for i in out.indices where i + 1 < out.count {
            out[i].floor = min(out[i].baseline, out[i + 1].baseline + halfStroke + 1)
        }
        out.reverse()
    }
}
