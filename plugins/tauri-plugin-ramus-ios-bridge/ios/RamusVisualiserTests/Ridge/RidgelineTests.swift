// Ported from ramusTV `RamusTVTests/Visualiser/RidgelineTests.swift`.

import XCTest
@testable import RamusVisualiser

/// Row maths from `Ridgeline.swift` (ramus `ui/src/lib/ridgeline.ts`).
final class RidgelineTests: XCTestCase {
    private let eps: Float = 1e-6

    private func assertEqual(
        _ a: [Float], _ b: [Float], accuracy: Float = 1e-6,
        file: StaticString = #filePath, line: UInt = #line
    ) {
        XCTAssertEqual(a.count, b.count, "length", file: file, line: line)
        for i in 0..<min(a.count, b.count) {
            XCTAssertEqual(a[i], b[i], accuracy: accuracy, "index \(i)", file: file, line: line)
        }
    }

    // MARK: mixBands

    func testMixAveragesChannelsIntoZeroToOne() {
        var out = [Float](repeating: -1, count: 2)
        mixBands([255, 51, 255, 153], channels: 2, into: &out)
        assertEqual(out, [1, 0.4])
    }

    func testMixZeroesOnLengthMismatch() {
        var out = [Float](repeating: 7, count: 3)
        mixBands([255, 51, 255, 153], channels: 2, into: &out)
        assertEqual(out, [0, 0, 0])
    }

    func testMixZeroesWithoutChannels() {
        var out = [Float](repeating: 7, count: 2)
        mixBands([255, 255], channels: 0, into: &out)
        assertEqual(out, [0, 0])
    }

    // MARK: smoothRow

    func testSmoothZeroCopies() {
        var out = [Float](repeating: 0, count: 3)
        smoothRow([0.2, 0.9, 0.4], into: &out, amount: 0)
        assertEqual(out, [0.2, 0.9, 0.4])
    }

    func testSmoothOneTakesNeighbourMeanAndEndsReuseThemselves() {
        var out = [Float](repeating: 0, count: 3)
        smoothRow([0, 1, 0], into: &out, amount: 1)
        assertEqual(out, [0.5, 0, 0.5])
    }

    func testSmoothBlendsBySplitAmount() {
        var out = [Float](repeating: 0, count: 3)
        smoothRow([0, 1, 0], into: &out, amount: 0.1)
        assertEqual(out, [0.05, 0.9, 0.05])
    }

    // MARK: spreadRow

    func testSpreadZeroSigmaCopies() {
        var out = [Float](repeating: 0, count: 3)
        spreadRow([0.1, 0.5, 0.2], into: &out, sigma: 0)
        assertEqual(out, [0.1, 0.5, 0.2])
    }

    func testSpreadKeepsPeakAndLiftsFlanksByTheBell() {
        var out = [Float](repeating: 0, count: 5)
        spreadRow([0, 0, 1, 0, 0], into: &out, sigma: 1)
        let e2 = Float(exp(-2.0)), e05 = Float(exp(-0.5))
        assertEqual(out, [e2, e05, 1, e05, e2])
    }

    func testSpreadTakesTheMaximumNotTheSum() {
        var out = [Float](repeating: 0, count: 3)
        spreadRow([1, 0, 1], into: &out, sigma: 1)
        assertEqual(out, [1, Float(exp(-0.5)), 1])
    }

    func testSpreadKeepsAPlateauFlat() {
        var out = [Float](repeating: 0, count: 5)
        spreadRow([0.5, 0.5, 0.5, 0.5, 0.5], into: &out, sigma: 2)
        assertEqual(out, [0.5, 0.5, 0.5, 0.5, 0.5])
    }

    func testSpreadRadiusStopsAtTheRowLength() {
        var out = [Float](repeating: 0, count: 2)
        spreadRow([1, 0], into: &out, sigma: 5)
        assertEqual(out, [1, Float(exp(-1.0 / 50.0))])
    }

    func testSpreadRebuildsItsBellWhenSigmaChanges() {
        let scratch = RidgeScratch()
        var a = [Float](repeating: 0, count: 5)
        spreadRow([0, 0, 1, 0, 0], into: &a, sigma: 1, scratch: scratch)
        spreadRow([0, 0, 1, 0, 0], into: &a, sigma: 0.5, scratch: scratch)
        var b = [Float](repeating: 0, count: 5)
        spreadRow([0, 0, 1, 0, 0], into: &b, sigma: 0.5)
        assertEqual(a, b)
    }

    func testSpreadHandlesEmptyAndSinglePointRows() {
        var empty: [Float] = []
        spreadRow([], into: &empty, sigma: 1)
        XCTAssertEqual(empty, [])
        var one = [Float](repeating: 0, count: 1)
        spreadRow([0.3], into: &one, sigma: 1)
        assertEqual(one, [0.3])
    }

    // MARK: grain

    func testGrainZeroCopies() {
        var out = [Float](repeating: 0, count: 2)
        applyGrain([0.4, 0.6], into: &out, noise: [1, -1], amount: 0)
        assertEqual(out, [0.4, 0.6])
    }

    func testGrainIsMultiplicativeSoFlatStaysFlat() {
        var out = [Float](repeating: 0, count: 3)
        applyGrain([0, 0.5, 1], into: &out, noise: [1, 1, -0.5], amount: 0.1)
        assertEqual(out, [0, 0.55, 0.95])
    }

    func testGrainClampsAtZero() {
        var out = [Float](repeating: 0, count: 1)
        applyGrain([0.5], into: &out, noise: [-1], amount: 2)
        assertEqual(out, [0])
    }

    func testRerollFillsMinusOneToOne() {
        var noise = [Float](repeating: 9, count: 1000)
        var rng = RidgeRandom(seed: 42)
        rerollGrain(&noise, using: &rng)
        XCTAssertTrue(noise.allSatisfy { $0 >= -1 && $0 < 1 })
        XCTAssertLessThan(noise.min()!, -0.9)
        XCTAssertGreaterThan(noise.max()!, 0.9)
    }

    func testSeededRandomIsRepeatable() {
        var a = RidgeRandom(seed: 7), b = RidgeRandom(seed: 7)
        XCTAssertEqual((0..<8).map { _ in a.next() }, (0..<8).map { _ in b.next() })
    }

    // MARK: edgeWindow

    func testEdgeWindowZeroTaperIsFlat() {
        assertEqual(edgeWindow(count: 4, taper: 0), [1, 1, 1, 1])
        assertEqual(edgeWindow(count: 4, taper: -1), [1, 1, 1, 1])
    }

    func testEdgeWindowRampsBothEndsWithARaisedCosine() {
        // ramp = round(0.2 * 11) = 2 points.
        assertEqual(edgeWindow(count: 11, taper: 0.2), [0, 0.5, 1, 1, 1, 1, 1, 1, 1, 0.5, 0])
    }

    func testEdgeWindowRoundsTheRampHalfUp() {
        // ramp = round(0.25 * 10) = 3 points.
        let w = edgeWindow(count: 10, taper: 0.25)
        let quarter = Float(0.5 - 0.5 * cos(Double.pi / 3))
        let third = Float(0.5 - 0.5 * cos(2 * Double.pi / 3))
        assertEqual(w, [0, quarter, third, 1, 1, 1, 1, third, quarter, 0])
    }

    // MARK: RidgeHistory

    func testHistoryIsSilentUntilPushed() {
        let h = RidgeHistory(rows: 3, width: 2)
        assertEqual(h.get(0), [0, 0])
        assertEqual(h.get(2), [0, 0])
    }

    func testHistoryIsNewestFirstAndDropsTheOldest() {
        let h = RidgeHistory(rows: 2, width: 1)
        h.push([1])
        h.push([2])
        assertEqual(h.get(0), [2])
        assertEqual(h.get(1), [1])
        h.push([3])
        assertEqual(h.get(0), [3])
        assertEqual(h.get(1), [2])
    }

    func testHistoryCutsWideRowsAndPadsNarrowOnes() {
        let h = RidgeHistory(rows: 2, width: 3)
        h.push([1, 2, 3, 4])
        assertEqual(h.get(0), [1, 2, 3])
        h.push([5])
        assertEqual(h.get(0), [5, 0, 0])
    }

    func testHistoryIsSilentOutsideItsRows() {
        let h = RidgeHistory(rows: 2, width: 1)
        h.push([1])
        h.push([2])
        assertEqual(h.get(-1), [0])
        assertEqual(h.get(2), [0])
    }

    func testHistoryClearsToSilence() {
        let h = RidgeHistory(rows: 2, width: 1)
        h.push([1])
        h.push([2])
        h.clear()
        assertEqual(h.get(0), [0])
        assertEqual(h.get(1), [0])
        h.push([4])
        assertEqual(h.get(0), [4])
        assertEqual(h.get(1), [0])
    }

    func testHistoryHoldsAtLeastOneRow() {
        let h = RidgeHistory(rows: 0, width: 1)
        XCTAssertEqual(h.rows, 1)
        h.push([3])
        assertEqual(h.get(0), [3])
    }

    // MARK: ridgeRow

    func testRidgeRowFrontAndBack() {
        let p = RidgeParams.standard
        let front = ridgeRow(0, rows: 51, height: 1000, params: p)
        XCTAssertEqual(front.baseline, 990, accuracy: 1e-9)
        XCTAssertEqual(front.scale, 600, accuracy: 1e-9)
        XCTAssertEqual(front.alpha, 0.85, accuracy: 1e-9)
        let back = ridgeRow(50, rows: 51, height: 1000, params: p)
        XCTAssertEqual(back.baseline, 369, accuracy: 1e-9)
        XCTAssertEqual(back.scale, 236.4, accuracy: 1e-9)
        XCTAssertEqual(back.alpha, 0, accuracy: 1e-9)
    }

    func testRidgeRowFadesAlongTheCurve() {
        let mid = ridgeRow(25, rows: 51, height: 1000, params: .standard)
        XCTAssertEqual(mid.alpha, 0.85 * pow(0.5, 0.75), accuracy: 1e-9)
        XCTAssertEqual(mid.baseline, 990 - 310.5, accuracy: 1e-9)
    }

    func testRidgeRowSingleRowIsTheFront() {
        let g = ridgeRow(0, rows: 1, height: 100, params: .standard)
        XCTAssertEqual(g.baseline, 99, accuracy: 1e-9)
        XCTAssertEqual(g.alpha, 0.85, accuracy: 1e-9)
    }

    // MARK: monotoneTangents

    func testTangentsOfALineAreItsSlope() {
        var m = [Float](repeating: 9, count: 4)
        monotoneTangents([0, 1, 2, 3], into: &m)
        assertEqual(m, [1, 1, 1, 1])
    }

    func testTangentsAreLevelAtAnExtremum() {
        var m = [Float](repeating: 9, count: 3)
        monotoneTangents([0, 1, 0], into: &m)
        assertEqual(m, [1, 0, -1])
    }

    func testTangentsAreLevelAtBothEndsOfAFlatSegment() {
        var m = [Float](repeating: 9, count: 3)
        monotoneTangents([1, 1, 2], into: &m)
        assertEqual(m, [0, 0, 1])
    }

    func testTangentsStayInsideTheMonotoneCircle() {
        let y: [Float] = [0, 0.1, 1, 1.01, 0.2, 0.2, 0.9, 0, 0.05, 1]
        var m = [Float](repeating: 0, count: y.count)
        monotoneTangents(y, into: &m)
        for k in 0..<(y.count - 1) {
            let d = y[k + 1] - y[k]
            if d == 0 {
                XCTAssertEqual(m[k], 0)
                XCTAssertEqual(m[k + 1], 0)
            } else {
                let a = Double(m[k] / d), b = Double(m[k + 1] / d)
                XCTAssertLessThanOrEqual(a * a + b * b, 9 + 1e-4, "segment \(k)")
            }
        }
    }

    func testTangentsOfShortRows() {
        var one = [Float](repeating: 9, count: 1)
        monotoneTangents([0.5], into: &one)
        assertEqual(one, [0])
        var none: [Float] = []
        monotoneTangents([], into: &none)
        XCTAssertEqual(none, [])
    }

    // MARK: axisSamples

    func testAxisLinearCurveSpreadsEvenly() {
        var at = [Float](repeating: 0, count: 9)
        axisSamples(bands: 5, curve: 1, into: &at)
        assertEqual(at, [0, 0.5, 1, 1.5, 2, 2.5, 3, 3.5, 4])
    }

    func testAxisCurveBelowOneWidensTheLowEndAndKeepsTheEnds() {
        var at = [Float](repeating: 0, count: 9)
        axisSamples(bands: 5, curve: 0.75, into: &at)
        XCTAssertEqual(at.first!, 0)
        XCTAssertEqual(at.last!, 4, accuracy: eps)
        XCTAssertEqual(at[4], Float(pow(0.5, 1 / 0.75) * 4), accuracy: eps)
        for j in 1..<8 { XCTAssertLessThan(at[j], Float(j) * 0.5) }
    }

    func testAxisNonPositiveCurveIsLinear() {
        var at = [Float](repeating: 0, count: 3)
        axisSamples(bands: 3, curve: 0, into: &at)
        assertEqual(at, [0, 1, 2])
    }

    func testAxisDegenerateSizes() {
        var one = [Float](repeating: 9, count: 1)
        axisSamples(bands: 64, curve: 0.75, into: &one)
        assertEqual(one, [0])
        var at = [Float](repeating: 9, count: 3)
        axisSamples(bands: 0, curve: 1, into: &at)
        assertEqual(at, [0, 0, 0])
    }

    // MARK: resampleRow

    func testResamplePassesThroughTheBands() {
        let src: [Float] = [0, 0.3, 1, 0.2, 0.2]
        var out = [Float](repeating: 0, count: 5)
        resampleRow(src, at: [0, 1, 2, 3, 4], into: &out)
        assertEqual(out, src)
    }

    func testResampleOfALineIsLinear() {
        var out = [Float](repeating: 0, count: 3)
        resampleRow([0, 1, 2, 3], at: [0.5, 1.5, 2.25], into: &out)
        assertEqual(out, [0.5, 1.5, 2.25])
    }

    func testResampleNeverOvershootsAPeak() {
        let src: [Float] = [0, 0, 1, 0.9, 0, 0]
        var at = [Float](repeating: 0, count: 51)
        axisSamples(bands: src.count, curve: 1, into: &at)
        var out = [Float](repeating: 0, count: at.count)
        resampleRow(src, at: at, into: &out)
        for (j, v) in out.enumerated() {
            let c = Double(at[j])
            let i = min(src.count - 2, Int(c.rounded(.down)))
            let lo = min(src[i], src[i + 1]), hi = max(src[i], src[i + 1])
            XCTAssertGreaterThanOrEqual(v, lo - eps, "point \(j)")
            XCTAssertLessThanOrEqual(v, hi + eps, "point \(j)")
        }
    }

    func testResampleClampsPositionsToTheRow() {
        var out = [Float](repeating: 0, count: 2)
        resampleRow([0.25, 0.75], at: [-3, 9], into: &out)
        assertEqual(out, [0.25, 0.75])
    }

    func testResampleDegenerateRows() {
        var out = [Float](repeating: 9, count: 3)
        resampleRow([0.4], at: [0, 0.5, 1], into: &out)
        assertEqual(out, [0.4, 0.4, 0.4])
        resampleRow([], at: [0, 0.5, 1], into: &out)
        assertEqual(out, [0, 0, 0])
        out = [9, 9, 9]
        resampleRow([0, 1], at: [0, 1], into: &out)
        assertEqual(out, [0, 0, 0])
    }

    // MARK: jsRound

    func testJSRoundRoundsHalvesUp() {
        XCTAssertEqual(jsRound(2.5), 3)
        XCTAssertEqual(jsRound(-2.5), -2)
        XCTAssertEqual(jsRound(-2.6), -3)
        XCTAssertEqual(jsRound(196.75), 197)
    }
}
