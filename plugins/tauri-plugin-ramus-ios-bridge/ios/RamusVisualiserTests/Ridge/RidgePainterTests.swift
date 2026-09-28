// Ported from ramusTV `RamusTVTests/Visualiser/RidgePainterTests.swift`.

import XCTest
@testable import RamusVisualiser

/// A frame source that hands out one fixed frame and records every read.
final class StubRidgeSource: RidgeFrameSource {
    var isPlaying = true
    var frame: [UInt8]?
    private(set) var reads: [(now: Double, leadSec: Double)] = []

    init(frame: [UInt8]?) {
        self.frame = frame
    }

    /// Every band of both channels at `level`.
    static func flat(_ level: UInt8, bands: Int = 64) -> StubRidgeSource {
        StubRidgeSource(frame: [UInt8](repeating: level, count: bands * 2))
    }

    func bands(now: Double, leadSec: Double) -> [UInt8]? {
        reads.append((now, leadSec))
        return frame
    }
}

/// The paint loop from `RidgePainter` (ramus
/// `ui/src/components/FocusVisualizer.tsx`, the ridge half of `render`).
final class RidgePainterTests: XCTestCase {
    private let frameMs = 1000.0 / 60
    private let start = 10_000.0

    // MARK: level curve and easing

    func testLevelCurveCutsTheFloorRaisesToGammaAndClampsTheGain() {
        let p = RidgeParams.standard
        XCTAssertEqual(RidgePainter.shape(0.5, params: p), 0)
        // Float(0.6) sits a hair above the floor, as a Float32Array level does.
        XCTAssertEqual(RidgePainter.shape(0.6, params: p), 0, accuracy: 1e-12)
        XCTAssertEqual(RidgePainter.shape(0.59, params: p), 0)
        XCTAssertEqual(RidgePainter.shape(0.8, params: p), Float(pow(0.5, 4.0) * 1.15), accuracy: 1e-6)
        XCTAssertEqual(RidgePainter.shape(0.99, params: p), 1)
        XCTAssertEqual(RidgePainter.shape(1, params: p), 1)
    }

    func testLevelCurveIdentity() {
        var p = RidgeParams.standard
        p.ridgeFloorCut = 0
        p.ridgeGamma = 1
        p.ridgeGain = 1
        XCTAssertEqual(RidgePainter.shape(0.37, params: p), 0.37)
    }

    func testEaseAlphaIsTheFactorAtSixtyHertz() {
        XCTAssertEqual(RidgePainter.easeAlpha(0.67, dtMs: 1000.0 / 60), 0.67, accuracy: 1e-12)
        XCTAssertEqual(RidgePainter.easeAlpha(0.67, dtMs: 2000.0 / 60), 1 - 0.33 * 0.33, accuracy: 1e-12)
        XCTAssertEqual(RidgePainter.easeAlpha(0.67, dtMs: 0), 0)
    }

    func testEaseAlphaClampsLongHitches() {
        XCTAssertEqual(
            RidgePainter.easeAlpha(0.5, dtMs: 5000), RidgePainter.easeAlpha(0.5, dtMs: 100),
            accuracy: 1e-12)
        XCTAssertEqual(RidgePainter.easeAlpha(0.5, dtMs: 100), 1 - pow(0.5, 6), accuracy: 1e-12)
    }

    // MARK: reading the source

    func testReadsTheSourceWithTheSyncLeadPlusTheOffset() {
        let source = StubRidgeSource.flat(0)
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        painter.leadOffsetMs = 20
        _ = painter.step(now: start, source: source, height: 1080)
        XCTAssertEqual(source.reads.count, 1)
        XCTAssertEqual(source.reads[0].now, start)
        XCTAssertEqual(source.reads[0].leadSec, 0.065, accuracy: 1e-12)
    }

    func testDoesNotReadWhileNotPlaying() {
        let source = StubRidgeSource.flat(255)
        source.isPlaying = false
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        _ = painter.step(now: start, source: source, height: 1080)
        XCTAssertTrue(source.reads.isEmpty)
    }

    func testRisesWithTheAttackEasing() {
        let source = StubRidgeSource.flat(255)
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        // The first paint has no frame delta, so nothing moves yet.
        _ = painter.step(now: start, source: source, height: 1080)
        XCTAssertEqual(painter.levels.count, 64)
        XCTAssertEqual(painter.levels[32], 0)
        _ = painter.step(now: start + frameMs, source: source, height: 1080)
        XCTAssertEqual(painter.levels[32], 0.67, accuracy: 1e-5)
        _ = painter.step(now: start + 2 * frameMs, source: source, height: 1080)
        XCTAssertEqual(painter.levels[32], 0.67 + 0.33 * 0.67, accuracy: 1e-5)
    }

    func testDecaysToSilenceWithoutAFrame() {
        let source = StubRidgeSource.flat(255)
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        var now = start
        for _ in 0..<60 {
            _ = painter.step(now: now, source: source, height: 1080)
            now += frameMs
        }
        XCTAssertEqual(painter.levels[32], 1, accuracy: 1e-4)
        source.frame = nil
        _ = painter.step(now: now, source: source, height: 1080)
        XCTAssertEqual(painter.levels[32], 0.5, accuracy: 1e-4)
        now += frameMs
        _ = painter.step(now: now, source: source, height: 1080)
        XCTAssertEqual(painter.levels[32], 0.25, accuracy: 1e-4)
    }

    func testDecaysToSilenceWhenPaused() {
        let source = StubRidgeSource.flat(255)
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        var now = start
        for _ in 0..<60 {
            _ = painter.step(now: now, source: source, height: 1080)
            now += frameMs
        }
        source.isPlaying = false
        _ = painter.step(now: now, source: source, height: 1080)
        XCTAssertEqual(painter.levels[32], 0.5, accuracy: 1e-4)
    }

    func testANilSourceDecaysToo() {
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        XCTAssertTrue(painter.step(now: start, source: nil, height: 1080))
        XCTAssertEqual(painter.levels.count, 64)
        XCTAssertTrue(painter.levels.allSatisfy { $0 == 0 })
    }

    func testBuffersFollowTheBandCount() {
        let source = StubRidgeSource.flat(255, bands: 32)
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        _ = painter.step(now: start, source: source, height: 1080)
        _ = painter.step(now: start + frameMs, source: source, height: 1080)
        XCTAssertEqual(painter.levels.count, 32)
        XCTAssertGreaterThan(painter.levels[16], 0)
        source.frame = [UInt8](repeating: 255, count: 128)
        _ = painter.step(now: start + 2 * frameMs, source: source, height: 1080)
        XCTAssertEqual(painter.levels.count, 64)
        XCTAssertEqual(painter.levels[32], 0.67, accuracy: 1e-5, "restarts from silence")
    }

    func testLevelsRunThroughSpreadAndSmooth() {
        // One loud band: the level curve keeps it, the spread widens it and
        // the smoothing blends it, so its neighbours lift too.
        var frame = [UInt8](repeating: 0, count: 128)
        frame[20] = 255
        frame[64 + 20] = 255
        let source = StubRidgeSource(frame: frame)
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        var now = start
        for _ in 0..<120 {
            _ = painter.step(now: now, source: source, height: 1080)
            now += frameMs
        }
        let e05 = exp(-0.5), e2 = exp(-2.0)
        XCTAssertEqual(Double(painter.levels[20]), 0.9 + 0.05 * 2 * e05, accuracy: 1e-4)
        XCTAssertEqual(Double(painter.levels[21]), 0.9 * e05 + 0.05 * (1 + e2), accuracy: 1e-4)
        XCTAssertEqual(Double(painter.levels[40]), 0, accuracy: 1e-6)
    }

    // MARK: history

    func testRowCadenceKeepsItsFractionalCredit() {
        XCTAssertNil(RidgePainter.nextRowAt(now: 1030, lastRowAt: 1000, periodMs: 33))
        XCTAssertEqual(RidgePainter.nextRowAt(now: 1034, lastRowAt: 1000, periodMs: 33), 1033)
        XCTAssertEqual(RidgePainter.nextRowAt(now: 1066, lastRowAt: 1000, periodMs: 33), 1033)
    }

    func testRowCadenceResyncsAfterAHitch() {
        XCTAssertEqual(RidgePainter.nextRowAt(now: 1067, lastRowAt: 1000, periodMs: 33), 1067)
        XCTAssertEqual(RidgePainter.nextRowAt(now: 5000, lastRowAt: 0, periodMs: 33), 5000)
    }

    func testRowPeriodRoundsToWholeDisplayFrames() {
        XCTAssertEqual(RidgePainter.rowPeriodMs(rowMs: 33, framePeriodMs: nil), 33)
        XCTAssertEqual(RidgePainter.rowPeriodMs(rowMs: 33, framePeriodMs: 16.68), 33.36, accuracy: 1e-9)
        XCTAssertEqual(RidgePainter.rowPeriodMs(rowMs: 33, framePeriodMs: 8.34), 33.36, accuracy: 1e-9)
        XCTAssertEqual(RidgePainter.rowPeriodMs(rowMs: 33, framePeriodMs: 20), 40, accuracy: 1e-9)
        XCTAssertEqual(RidgePainter.rowPeriodMs(rowMs: 33, framePeriodMs: 50), 50, accuracy: 1e-9)
    }

    /// 33 ms rows on a 60 Hz display: every row stays up for exactly two
    /// frames, even with the callback time jittering around the vsync.
    func testRowsLandOnAFixedFrameCadence() {
        let source = StubRidgeSource.flat(255)
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        let period = 16.68
        painter.framePeriodMs = period
        var cutFrames: [Int] = []
        var last = painter.rowsCut
        for frame in 0..<600 {
            let jitter = [0.4, -0.3, 0.1, 0.5, -0.5, 0.2][frame % 6]
            _ = painter.step(now: start + Double(frame) * period + jitter, source: source, height: 1080)
            if painter.rowsCut != last {
                cutFrames.append(frame)
                last = painter.rowsCut
            }
        }
        let gaps = Set(zip(cutFrames.dropFirst(), cutFrames).map { $0 - $1 })
        XCTAssertEqual(gaps, [2])
    }

    func testFrameIsNilBeforeTheFirstPaint() {
        XCTAssertNil(RidgePainter(random: RidgeRandom(seed: 1)).frame())
    }

    func testFrameCarriesTheLiveRowAndTheHistory() throws {
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        var now = start
        for _ in 0..<30 {
            _ = painter.step(now: now, source: StubRidgeSource.flat(255), height: 1080)
            now += frameMs
        }
        let frame = try XCTUnwrap(painter.frame())
        let history = try XCTUnwrap(painter.history)
        XCTAssertEqual(frame.rows, 51)
        XCTAssertEqual(frame.historyRows, 51)
        XCTAssertEqual(frame.edge.count, 63 * 5 + 1)
        XCTAssertEqual(frame.levels(0).count, 63 * 5 + 1)
        XCTAssertEqual(frame.levels(1), history.get(0))
        XCTAssertEqual(frame.levels(5), history.get(4))
        XCTAssertEqual(frame.levels(60), [Float](repeating: 0, count: 63 * 5 + 1), "past the history is silence")
        XCTAssertEqual(frame.layout, painter.layout)
        XCTAssertEqual(frame.color, .white)
    }

    func testGlidePhaseCountsWholeFramesSinceTheCut() {
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        painter.framePeriodMs = 16.68
        var last = painter.rowsCut
        for frame in 0..<120 {
            _ = painter.step(now: start + Double(frame) * 16.68, source: StubRidgeSource.flat(255), height: 1080)
            guard let f = painter.frame() else { return XCTFail("no frame at \(frame)") }
            XCTAssertEqual(f.framesPerRow, 2)
            XCTAssertEqual(f.framesSinceCut, painter.rowsCut != last ? 0 : 1, "frame \(frame)")
            last = painter.rowsCut
        }
    }

    func testGlidePhaseAt120Hz() {
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        painter.framePeriodMs = 8.34
        var phases: [Int] = []
        for frame in 0..<40 {
            _ = painter.step(now: start + Double(frame) * 8.34, source: StubRidgeSource.flat(255), height: 1080)
            phases.append(painter.frame()?.framesSinceCut ?? -1)
        }
        XCTAssertEqual(painter.frame()?.framesPerRow, 4)
        let tail = Array(phases.suffix(12))
        XCTAssertEqual(Set(tail), [0, 1, 2, 3])
        for (a, b) in zip(tail, tail.dropFirst()) {
            XCTAssertEqual(b, (a + 1) % 4, "phases step by one frame: \(tail)")
        }
    }

    func testGlidePhaseWithoutAFramePeriod() {
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        _ = painter.step(now: start, source: StubRidgeSource.flat(255), height: 1080)
        _ = painter.step(now: start + frameMs, source: StubRidgeSource.flat(255), height: 1080)
        XCTAssertEqual(painter.frame()?.framesPerRow, 1)
        XCTAssertEqual(painter.frame()?.framesSinceCut, 0)
    }

    /// A callback 9 ms late on the frame that cuts a row doesn't move the
    /// glide: the phase counts display frames (vsync times), not callback
    /// times.
    func testGlidePhaseFollowsDisplayFramesNotCallbackTimes() {
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        let period = 16.68
        painter.framePeriodMs = period
        var phases: [Int] = []
        for (k, delay) in [0, 0, 9, 0, 0.0].enumerated() {
            let vsync = start + Double(k) * period
            _ = painter.step(now: vsync + delay, frameTime: vsync, source: StubRidgeSource.flat(255), height: 1080)
            phases.append(painter.frame()?.framesSinceCut ?? -1)
        }
        XCTAssertEqual(phases, [0, 1, 0, 1, 0])
    }

    func testCutsARowEveryRowMsOfWallClock() {
        let source = StubRidgeSource.flat(255)
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        var now = start
        for _ in 0..<61 {
            _ = painter.step(now: now, source: source, height: 1080)
            now += frameMs
        }
        // One second at 60 Hz after the first cut: 1000 / 33 rows.
        XCTAssertEqual(painter.rowsCut, 1 + 30)
    }

    func testHistoryCarriesTheLiveLineUpTheStack() throws {
        let source = StubRidgeSource.flat(255)
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        var now = start
        for _ in 0..<30 {
            _ = painter.step(now: now, source: source, height: 1080)
            now += frameMs
        }
        let history = try XCTUnwrap(painter.history)
        XCTAssertEqual(history.rows, 51)
        XCTAssertEqual(history.width, 63 * 5 + 1)
        // Rows are cut on every other frame, eased then textured: the first
        // before the line had risen, the next at 0.67 + 0.33 * 0.67, the
        // newest at full height; grain moves each by up to 3 %.
        let mid = history.width / 2
        XCTAssertEqual(history.get(0)[mid], 1, accuracy: 0.031)
        XCTAssertEqual(history.get(13)[mid], 0.8911, accuracy: 0.8911 * 0.031)
        XCTAssertEqual(history.get(14)[mid], 0)
        XCTAssertEqual(history.get(20)[mid], 0, "not cut yet")
    }

    // MARK: settling

    func testSkipsThePaintOnceEveryRowIsFlat() {
        let source = StubRidgeSource.flat(0)
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        // Quiet from the first paint: the stack takes (51 + 1) * 33 ms to
        // carry the last row off the top.
        let settleMs = 52 * 33.0
        var now = start
        while now - start <= settleMs {
            XCTAssertTrue(painter.step(now: now, source: source, height: 1080), "at \(now - start)")
            now += frameMs
        }
        XCTAssertFalse(painter.step(now: now, source: source, height: 1080))
        XCTAssertFalse(painter.step(now: now + frameMs, source: source, height: 1080))
        let cut = painter.rowsCut
        XCTAssertFalse(painter.step(now: now + 200, source: source, height: 1080))
        XCTAssertEqual(painter.rowsCut, cut, "no rows are cut while settled")
    }

    func testAWipeForcesOnePaint() {
        let painter = settledPainter()
        painter.invalidate()
        XCTAssertTrue(painter.step(now: 20_000, source: nil, height: 1080))
        XCTAssertFalse(painter.step(now: 20_016, source: nil, height: 1080))
    }

    func testARowCountChangeForcesAPaint() {
        let painter = settledPainter()
        painter.fullScreen = true
        XCTAssertTrue(painter.step(now: 20_000, source: nil, height: 1080))
    }

    func testSoundWakesASettledStack() {
        let painter = settledPainter()
        let source = StubRidgeSource.flat(255)
        XCTAssertTrue(painter.step(now: 20_000, source: source, height: 1080))
        XCTAssertTrue(painter.step(now: 20_016, source: source, height: 1080))
    }

    func testAPointUnderHalfAPointOfRiseCountsAsFlat() {
        // Either side of the threshold on a 100 pt tall frame: rise = 100 * 0.6.
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        XCTAssertTrue(painter.isFlat(peak: 0.45 / 60, height: 100))
        XCTAssertFalse(painter.isFlat(peak: 0.55 / 60, height: 100))
    }

    private func settledPainter() -> RidgePainter {
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        var now = 10_000.0
        while now < 19_000 {
            _ = painter.step(now: now, source: nil, height: 1080)
            now += frameMs
        }
        XCTAssertFalse(painter.step(now: 19_990, source: nil, height: 1080))
        return painter
    }

    // MARK: layout

    func testStandardStackUsesTheRidgeRows() {
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        XCTAssertEqual(painter.rowCount, 51)
        XCTAssertEqual(painter.layout.ridgeHeight, 0.621)
    }

    func testFullScreenDeepensTheStackAtTheSameSpacing() {
        let painter = RidgePainter(random: RidgeRandom(seed: 1))
        painter.fullScreen = true
        XCTAssertEqual(painter.rowCount, 67)
        XCTAssertEqual(painter.layout.ridgeHeight, 0.621 * 66 / 50, accuracy: 1e-12)
        _ = painter.step(now: start, source: StubRidgeSource.flat(255), height: 1080)
        XCTAssertEqual(painter.history?.rows, 67)
    }

}
