// Ported from ramusTV `RamusTVTests/Visualiser/RidgeGeometryTests.swift`.

import XCTest
@testable import RamusVisualiser

/// `RidgeGeometryBuilder`: the erase spans and stroke segments a frame
/// draws.
final class RidgeGeometryTests: XCTestCase {
    private let flat11 = [Float](repeating: 0, count: 11)
    private let ones11 = [Float](repeating: 1, count: 11)
    private let bump: [Float] = [0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0]
    private let square = RidgeTarget(pixelWidth: 200, pixelHeight: 200, scale: 2)

    private func build(_ frame: RidgeFrame?, _ target: RidgeTarget, glide: Bool = false) -> RidgeDrawList {
        var list = RidgeDrawList()
        RidgeGeometryBuilder().build(frame, target: target, glide: glide, into: &list)
        return list
    }

    func testMemoryLayoutMatchesTheShaders() {
        XCTAssertEqual(MemoryLayout<RidgeEraseSpan>.stride, 40)
        XCTAssertEqual(MemoryLayout<RidgeEraseSpan>.offset(of: \.a), 0)
        XCTAssertEqual(MemoryLayout<RidgeEraseSpan>.offset(of: \.b), 8)
        XCTAssertEqual(MemoryLayout<RidgeEraseSpan>.offset(of: \.x0), 16)
        XCTAssertEqual(MemoryLayout<RidgeEraseSpan>.offset(of: \.floor), 32)
        XCTAssertEqual(MemoryLayout<RidgeStrokeSegment>.stride, 40)
        XCTAssertEqual(MemoryLayout<RidgeStrokeSegment>.offset(of: \.p3), 24)
        XCTAssertEqual(MemoryLayout<RidgeStrokeSegment>.offset(of: \.alpha), 32)
        XCTAssertEqual(MemoryLayout<RidgeStrokeSegment>.offset(of: \.flags), 36)
    }

    func testFlatRowsStrokeButEraseNothing() {
        let frame = makeRidgeFrame(rows: 5, params: .solid, edge: ones11) { _ in self.flat11 }
        let list = build(frame, square)
        XCTAssertEqual(list.rows.count, 5)
        XCTAssertTrue(list.spans.isEmpty)
        XCTAssertEqual(list.segments.count, 50)
        XCTAssertEqual(list.lineWidth, 2.5)
        let front = list.rows[4].segments
        XCTAssertEqual(front.count, 10)
        let first = list.segments[front.lowerBound], last = list.segments[front.upperBound - 1]
        XCTAssertEqual(first.flags, RidgeStrokeSegment.openStart)
        XCTAssertEqual(last.flags, RidgeStrokeSegment.openEnd)
        XCTAssertEqual(list.segments[front.lowerBound + 1].flags, 0)
        XCTAssertEqual(first.p1, SIMD2(0, 198.25))
        XCTAssertEqual(first.p2, SIMD2(20, 198.25))
        XCTAssertEqual(first.p0, first.p1, "no point before the row's start")
        XCTAssertEqual(last.p3, last.p2, "no point after the row's end")
        XCTAssertEqual(list.segments[front.lowerBound + 1].p0, first.p1)
    }

    func testEraseMeetsTheLineWhereItCrossesTheFloor() throws {
        // Two rows, 0.2 of the height apart: baselines 198.25 and 158.25 px,
        // the front floor 160.5 px. The bump rises 120 px to 78.25 over points
        // 3...7, 20 px apart.
        var p = RidgeParams.solid
        p.ridgeHeight = 0.2
        p.ridgeDepthScale = 1
        let frame = makeRidgeFrame(rows: 2, params: p, edge: ones11) { k in k == 0 ? self.bump : self.flat11 }
        let list = build(frame, square)
        XCTAssertEqual(list.rows.count, 2)
        XCTAssertTrue(list.rows[0].spans.isEmpty, "the back row erases nothing")
        let spans = Array(list.spans[list.rows[1].spans])
        XCTAssertEqual(spans.count, 6)
        let rise = try XCTUnwrap(spans.first)
        XCTAssertEqual(rise.x0, 40 + 20 * (37.75 / 120), accuracy: 1e-3, "enters at the floor crossing")
        XCTAssertEqual(rise.top0, 160.5, accuracy: 1e-4)
        XCTAssertEqual(rise.x1, 60, accuracy: 1e-4)
        XCTAssertEqual(rise.top1, 78.25, accuracy: 1e-4)
        XCTAssertEqual(rise.a, SIMD2(40, 198.25))
        XCTAssertEqual(rise.b, SIMD2(60, 78.25))
        XCTAssertEqual(spans[2].x0, 80, accuracy: 1e-4)
        XCTAssertEqual(spans[2].top0, 78.25, accuracy: 1e-4)
        let fall = try XCTUnwrap(spans.last)
        XCTAssertEqual(fall.x1, 140 + 20 * (82.25 / 120), accuracy: 1e-3, "leaves at the floor crossing")
        XCTAssertEqual(fall.top1, 160.5, accuracy: 1e-4)
        for span in spans { XCTAssertEqual(span.floor, 160.5, accuracy: 1e-4) }
    }

    func testZeroAlphaRowsStrokeNothing() {
        let frame = makeRidgeFrame(rows: 3, edge: ones11) { _ in self.flat11 }
        let list = build(frame, square)
        XCTAssertTrue(list.rows[0].segments.isEmpty, "the back row fades to alpha 0")
        XCTAssertEqual(list.rows[1].segments.count, 10)
    }

    func testEdgeWindowScalesEachPoint() {
        var edge = ones11
        edge[5] = 0
        let frame = makeRidgeFrame(rows: 1, params: .solid, edge: edge) { _ in self.ones11 }
        let list = build(frame, square)
        XCTAssertEqual(list.segments[4].p2, SIMD2(100, 198.25), "point 5 held at the baseline")
        XCTAssertEqual(list.segments[3].p2, SIMD2(80, 78.25))
    }

    func testRowsShorterThanTheEdgeReadAsSilence() {
        let frame = makeRidgeFrame(rows: 2, params: .solid, edge: ones11) { k in k == 0 ? [1, 1, 1] : [] }
        let list = build(frame, square)
        let front = Array(list.segments[list.rows[1].segments])
        XCTAssertEqual(front.count, 10)
        XCTAssertEqual(front[5].p1.y, 198.25, "point 5 is past the row's end")
        XCTAssertLessThan(front[0].p2.y, 198.25, "point 1 rises")
        XCTAssertTrue(list.segments[list.rows[0].segments].allSatisfy { $0.p1.y == $0.p2.y }, "an empty row is flat")
    }

    func testLinesAboveTheFrameStayFinite() {
        let frame = makeRidgeFrame(rows: 2, params: .solid, edge: ones11) { _ in [Float](repeating: 3, count: 11) }
        let list = build(frame, square)
        XCTAssertFalse(list.segments.isEmpty)
        for s in list.segments {
            XCTAssertTrue(s.p1.x.isFinite && s.p1.y.isFinite && s.p2.y.isFinite)
        }
        XCTAssertLessThan(list.segments[list.rows[1].segments.lowerBound].p2.y, 0, "above the top edge")
    }

    func testDegenerateInputsBuildNothing() {
        XCTAssertTrue(build(nil, square).rows.isEmpty)
        XCTAssertTrue(build(makeRidgeFrame(rows: 3, edge: [1]) { _ in [0] }, square).rows.isEmpty)
        XCTAssertTrue(build(makeRidgeFrame(rows: 0, edge: ones11) { _ in self.flat11 }, square).rows.isEmpty)
        let frame = makeRidgeFrame(rows: 3, edge: ones11) { _ in self.flat11 }
        XCTAssertTrue(build(frame, RidgeTarget(pixelWidth: 0, pixelHeight: 0, scale: 2)).rows.isEmpty)
    }

    func testRebuildingReusesTheBuffers() {
        let builder = RidgeGeometryBuilder()
        var list = RidgeDrawList()
        let frame = makeRidgeFrame(rows: 51, params: .solid, edge: ones11) { _ in self.bump }
        builder.build(frame, target: square, glide: false, into: &list)
        let capacities = (list.spans.capacity, list.segments.capacity, list.rows.capacity)
        builder.build(frame, target: square, glide: false, into: &list)
        XCTAssertEqual(list.spans.capacity, capacities.0)
        XCTAssertEqual(list.segments.capacity, capacities.1)
        XCTAssertEqual(list.rows.capacity, capacities.2)
    }

    func testGlideMovesTheHistoryBetweenCuts() {
        let rows = 51
        func frontOfRow2(_ q: Int) -> Float {
            let frame = makeRidgeFrame(rows: rows, edge: ones11, framesSinceCut: q, framesPerRow: 2) { _ in self.flat11 }
            let list = build(frame, RidgeTarget(pixelWidth: 3840, pixelHeight: 2160, scale: 2), glide: true)
            // Back to front, and history row 0 is hidden at q == 0: the
            // second row from the front is history row 1 at q 0 and history
            // row 0 at q 1.
            let draw = list.rows[list.rows.count - 2]
            return list.segments[draw.segments.lowerBound].p1.y
        }
        XCTAssertEqual(frontOfRow2(0), 2138.25 - 27)
        XCTAssertEqual(frontOfRow2(1), 2138.25 - 14)
    }

    func testFrameStatsReportPaintsPerSecond() {
        var stats = RidgeFrameStats()
        XCTAssertEqual(stats.paintsPerSecond, 0)
        stats.paintMs = [Double](repeating: 1, count: 120)
        stats.seconds = 2
        XCTAssertEqual(stats.paintsPerSecond, 60)
    }
}
