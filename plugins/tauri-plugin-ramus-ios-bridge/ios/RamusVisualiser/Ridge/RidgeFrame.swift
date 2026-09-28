// Ported from ramusTV `RamusTV/Visualiser/RidgeFrame.swift`.

import Foundation

/// Everything one paint of the ridge reads, taken from `RidgePainter` after
/// a `step`, for a renderer that builds its own geometry.
struct RidgeFrame {
    /// Rows in the stack, the live front row included.
    var rows: Int
    /// Levels of stack row `k`, one per fine-grid point: 0 is the live row,
    /// `k >= 1` history row `k - 1` (the newest first). A row the history
    /// doesn't hold reads as silence.
    var levels: (Int) -> [Float]
    /// History rows held: `levels(1...historyRows)` are real rows.
    var historyRows: Int
    /// Per-point edge multiplier (the taper window), one per fine-grid point.
    var edge: [Float]
    /// Stroke colour of every row.
    var color: RidgeRGB
    /// The layout the rows are drawn with (`RidgePainter.layout`).
    var layout: RidgeParams
    /// Whole display frames since the latest history row was cut, 0 up to
    /// `framesPerRow - 1`.
    var framesSinceCut = 0
    /// Display frames each history row lasts; 1 when the display's refresh
    /// period is unknown.
    var framesPerRow = 1
}
