// Ported from ramusTV `RamusTVTests/Visualiser/RidgeFrameFixtures.swift`.

@testable import RamusVisualiser

/// A frame whose stack row `k` reads `row(k)`, for renderer tests.
func makeRidgeFrame(
    rows: Int, historyRows: Int? = nil, params: RidgeParams = .standard, edge: [Float],
    framesSinceCut: Int = 0, framesPerRow: Int = 1, row: @escaping (Int) -> [Float]
) -> RidgeFrame {
    RidgeFrame(
        rows: rows, levels: row, historyRows: historyRows ?? rows, edge: edge, color: .white,
        layout: params, framesSinceCut: framesSinceCut, framesPerRow: framesPerRow)
}

extension RidgeParams {
    /// Every row fully opaque, so a covered pixel reads 255 whatever its depth.
    static var solid: RidgeParams {
        var p = RidgeParams.standard
        p.ridgeAlpha = 1
        p.ridgeBackAlpha = 1
        return p
    }
}
