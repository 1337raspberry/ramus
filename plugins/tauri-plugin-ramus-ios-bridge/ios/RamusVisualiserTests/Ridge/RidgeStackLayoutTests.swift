// Ported from ramusTV `RamusTVTests/Visualiser/RidgeStackLayoutTests.swift`.

import XCTest
@testable import RamusVisualiser

/// `RidgeStackLayout`: where each drawn row sits on the pixel grid.
final class RidgeStackLayoutTests: XCTestCase {
    private let flat11 = [Float](repeating: 0, count: 11)
    private let ones11 = [Float](repeating: 1, count: 11)

    private func place(_ frame: RidgeFrame, _ target: RidgeTarget, glide: Bool = false) -> [RidgeRowPlacement] {
        var out: [RidgeRowPlacement] = []
        RidgeStackLayout.place(frame, target: target, glide: glide, into: &out)
        return out
    }

    func testRidgeRowAtAWholeDepthMatchesTheRowIndex() {
        let p = RidgeParams.standard
        for k in [0, 10, 25, 50] {
            XCTAssertEqual(ridgeRow(depth: Double(k) / 50, height: 1080, params: p), ridgeRow(k, rows: 51, height: 1080, params: p))
        }
    }

    func testFlatStackSitsOnThePixelGrid() {
        // 100 pt tall at 2x: front baseline 99 pt, back 99 - 62.1 pt. Stroke
        // 2.5 px: front upper edge round(198 - 1.25) = 197, pitch
        // round(62.1 / 4 * 2) = 31 px, so baseline k is 198.25 - 31k.
        let frame = makeRidgeFrame(rows: 5, params: .solid, edge: ones11) { _ in self.flat11 }
        let rows = place(frame, RidgeTarget(pixelWidth: 200, pixelHeight: 200, scale: 2))
        XCTAssertEqual(rows.map(\.row), [4, 3, 2, 1, 0], "back to front")
        for placement in rows {
            XCTAssertEqual(placement.baseline, 198.25 - 31 * Double(placement.row), accuracy: 1e-9)
        }
    }

    func testFloorsSitAPixelBelowTheNextRowsStroke() {
        let frame = makeRidgeFrame(rows: 5, params: .solid, edge: ones11) { _ in self.flat11 }
        let rows = place(frame, RidgeTarget(pixelWidth: 200, pixelHeight: 200, scale: 2))
        XCTAssertNil(rows[0].floor, "the farthest row erases nothing")
        // Front row: the row behind has baseline 167.25; its stroke reaches
        // 1.25 px below that, and the floor is 1 px further.
        XCTAssertEqual(rows[4].floor ?? .nan, 169.5, accuracy: 1e-9)
        XCTAssertEqual(rows[3].floor ?? .nan, 138.5, accuracy: 1e-9)
    }

    func testRiseAndFadeFollowDepth() {
        let p = RidgeParams.standard
        let frame = makeRidgeFrame(rows: 3, params: p, edge: ones11) { _ in self.flat11 }
        let rows = place(frame, RidgeTarget(pixelWidth: 100, pixelHeight: 100, scale: 1))
        let middle = ridgeRow(1, rows: 3, height: 100, params: p)
        XCTAssertEqual(rows[1].row, 1)
        XCTAssertEqual(rows[1].alpha, middle.alpha, accuracy: 1e-12)
        XCTAssertEqual(rows[1].rise, middle.scale, accuracy: 1e-12)
    }

    func testSingleRowStackIsTheLiveRowAlone() {
        let frame = makeRidgeFrame(rows: 1, params: .solid, edge: ones11) { _ in self.flat11 }
        let rows = place(frame, RidgeTarget(pixelWidth: 200, pixelHeight: 200, scale: 2))
        XCTAssertEqual(rows.map(\.row), [0])
        XCTAssertNil(rows[0].floor)
    }

    func testEmptyTargetPlacesNothing() {
        let frame = makeRidgeFrame(rows: 5, edge: ones11) { _ in self.flat11 }
        XCTAssertTrue(place(frame, RidgeTarget(pixelWidth: 0, pixelHeight: 200, scale: 2)).isEmpty)
        XCTAssertTrue(place(frame, RidgeTarget(pixelWidth: 200, pixelHeight: 0, scale: 2)).isEmpty)
        XCTAssertTrue(place(makeRidgeFrame(rows: 0, edge: ones11) { _ in self.flat11 },
                            RidgeTarget(pixelWidth: 200, pixelHeight: 200, scale: 2)).isEmpty)
    }

    // MARK: glide

    /// 4K: frontY 1069.2 pt, backY 398.52 pt; front upper edge
    /// round(2138.4 - 1.25) = 2137, baseline 2138.25; pitch round(26.83) = 27.
    private let uhd = RidgeTarget(pixelWidth: 3840, pixelHeight: 2160, scale: 2)
    private let front = 2138.25

    private func stack(framesSinceCut: Int, framesPerRow: Int = 2, glide: Bool = true) -> [RidgeRowPlacement] {
        let frame = makeRidgeFrame(
            rows: 51, edge: ones11, framesSinceCut: framesSinceCut, framesPerRow: framesPerRow
        ) { _ in self.flat11 }
        return place(frame, uhd, glide: glide)
    }

    func testGlideOffMatchesTheFixedSlots() {
        let rows = stack(framesSinceCut: 1, glide: false)
        XCTAssertEqual(rows.count, 51)
        for placement in rows {
            XCTAssertEqual(placement.baseline, front - 27 * Double(placement.row), accuracy: 1e-9)
        }
    }

    func testGlideShiftsEveryHistoryRowByTheSameWholePixels() {
        // Halfway through a two-frame row: every history row is round(13.5)
        // = 14 px above its slot at the cut.
        let rows = stack(framesSinceCut: 1)
        XCTAssertEqual(rows.count, 51)
        for placement in rows where placement.row > 0 {
            let j = Double(placement.row - 1)
            XCTAssertEqual(placement.baseline, front - 27 * j - 14, accuracy: 1e-9, "row \(placement.row)")
        }
        XCTAssertEqual(rows.last?.baseline ?? .nan, front, accuracy: 1e-9, "the live row stays put")
    }

    func testGlideHidesTheNewestRowAtTheCut() {
        let rows = stack(framesSinceCut: 0)
        XCTAssertFalse(rows.contains { $0.row == 1 }, "the newest row coincides with the live row")
        let second = try? XCTUnwrap(rows.first { $0.row == 2 })
        XCTAssertEqual(second?.baseline ?? .nan, front - 27, accuracy: 1e-9)
        // The live row's floor comes from the next drawn row behind it.
        XCTAssertEqual(rows.last?.floor ?? .nan, front - 27 + 1.25 + 1, accuracy: 1e-9)
    }

    func testGlideCarriesTheOldestRowPastTheBack() {
        XCTAssertTrue(stack(framesSinceCut: 0).contains { $0.row == 51 }, "at the back slot, depth 1")
        let moving = stack(framesSinceCut: 1)
        XCTAssertFalse(moving.contains { $0.row == 51 }, "past the back")
        XCTAssertTrue(moving.contains { $0.row == 50 })
    }

    func testGlideFadesContinuouslyAcrossACut() {
        func alpha(_ rows: [RidgeRowPlacement], _ row: Int) -> Double { rows.first { $0.row == row }?.alpha ?? .nan }
        let atCut = stack(framesSinceCut: 0), halfway = stack(framesSinceCut: 1)
        // History row 10 (stack row 11) moves from depth 10/50 to 10.5/50,
        // then becomes history row 11 at depth 11/50.
        XCTAssertGreaterThan(alpha(atCut, 11), alpha(halfway, 11))
        XCTAssertGreaterThan(alpha(halfway, 11), alpha(atCut, 12))
        XCTAssertEqual(alpha(halfway, 11), ridgeRow(depth: 10.5 / 50, height: 1080, params: .standard).alpha, accuracy: 1e-12)
    }

    func testGlideAt120HzShiftsAQuarterPitchPerFrame() {
        let shifts = (0..<4).map { q -> Double in
            let rows = stack(framesSinceCut: q, framesPerRow: 4)
            let second = rows.first { $0.row == 3 }!.baseline  // history row 2
            return front - 27 * 2 - second
        }
        XCTAssertEqual(shifts, [0, 7, 14, 20], "round(27 * q / 4)")
    }
}
