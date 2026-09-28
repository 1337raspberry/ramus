// Ported from ramusTV `RamusTVTests/Tap/SpectrumRingTests.swift`.

import XCTest
@testable import RamusVisualiser

/// Behaviour of the port of `ui/src/lib/spectrumRing.ts` (which has no
/// tests of its own), pinned against the TypeScript source.
final class SpectrumRingTests: XCTestCase {
    /// Two bands per channel, two channels: four values per frame.
    private let bandCount = 2
    private let channels = 2

    private func frame(_ pos: Double, _ value: UInt8) -> SpectrumFrame {
        SpectrumFrame(pos: pos, bands: [UInt8](repeating: value, count: bandCount * channels))
    }

    private func push(_ ring: SpectrumRing, epoch: UInt64, _ frames: [SpectrumFrame], now: Double = 0) {
        ring.push(epoch: epoch, bandCount: bandCount, channels: channels, frames: frames, now: now)
    }

    // MARK: - Push and pick

    func testPicksTheClosestFrameWithinTheWindow() {
        let ring = SpectrumRing()
        push(ring, epoch: 1, [frame(1.00, 10), frame(1.02, 20), frame(1.04, 30)])
        XCTAssertEqual(ring.pickSpectrumFrame(epoch: 1, target: 1.021, lagSec: 0.25, leadSec: 0.05)?.bands.first, 20)
        XCTAssertEqual(ring.pickSpectrumFrame(epoch: 1, target: 1.035, lagSec: 0.25, leadSec: 0.05)?.bands.first, 30)
    }

    func testTheWindowIsLagBehindAndLeadAhead() {
        let ring = SpectrumRing()
        push(ring, epoch: 1, [frame(1.0, 10)])
        // A frame just inside 0.25 s behind the target is still drawn,
        // 0.26 s is not.
        XCTAssertNotNil(ring.pickSpectrumFrame(epoch: 1, target: 1.249, lagSec: 0.25, leadSec: 0.05))
        XCTAssertNil(ring.pickSpectrumFrame(epoch: 1, target: 1.26, lagSec: 0.25, leadSec: 0.05))
        // Just inside 0.05 s ahead is drawn, 0.06 s is not.
        XCTAssertNotNil(ring.pickSpectrumFrame(epoch: 1, target: 0.951, lagSec: 0.25, leadSec: 0.05))
        XCTAssertNil(ring.pickSpectrumFrame(epoch: 1, target: 0.94, lagSec: 0.25, leadSec: 0.05))
    }

    func testPicksOnlyFromTheRequestedEpochOrAnyWhenNil() {
        let ring = SpectrumRing()
        push(ring, epoch: 1, [frame(0.5, 10)])
        push(ring, epoch: 2, [frame(0.5, 20)])
        XCTAssertEqual(ring.pickSpectrumFrame(epoch: 1, target: 0.5, lagSec: 0.25, leadSec: 0.05)?.bands.first, 10)
        XCTAssertEqual(ring.pickSpectrumFrame(epoch: 2, target: 0.5, lagSec: 0.25, leadSec: 0.05)?.bands.first, 20)
        XCTAssertNil(ring.pickSpectrumFrame(epoch: 3, target: 0.5, lagSec: 0.25, leadSec: 0.05))
        XCTAssertNotNil(ring.pickSpectrumFrame(epoch: nil, target: 0.5, lagSec: 0.25, leadSec: 0.05))
    }

    func testDropsFramesThatDisagreeWithTheStatedShape() {
        let ring = SpectrumRing()
        let wrong = SpectrumFrame(pos: 1.0, bands: [1, 2, 3])
        push(ring, epoch: 1, [wrong, frame(2.0, 9)])
        XCTAssertNil(ring.pickSpectrumFrame(epoch: 1, target: 1.0, lagSec: 0.1, leadSec: 0.1))
        XCTAssertNotNil(ring.pickSpectrumFrame(epoch: 1, target: 2.0, lagSec: 0.1, leadSec: 0.1))
    }

    func testHoldsTheNewest128Frames() {
        let ring = SpectrumRing()
        let frames = (0..<130).map { frame(Double($0) / 60, UInt8($0 % 256)) }
        push(ring, epoch: 1, frames)
        XCTAssertNil(ring.pickSpectrumFrame(epoch: 1, target: 0, lagSec: 0.001, leadSec: 0.001))
        XCTAssertNil(ring.pickSpectrumFrame(epoch: 1, target: 1.0 / 60, lagSec: 0.001, leadSec: 0.001))
        XCTAssertEqual(ring.pickSpectrumFrame(epoch: 1, target: 2.0 / 60, lagSec: 0.001, leadSec: 0.001)?.bands.first, 2)
        XCTAssertEqual(ring.pickSpectrumFrame(epoch: 1, target: 129.0 / 60, lagSec: 0.001, leadSec: 0.001)?.bands.first, 129)
    }

    func testLastPushAtStampsEveryPush() {
        let ring = SpectrumRing()
        XCTAssertEqual(ring.lastPushAt, 0)
        push(ring, epoch: 1, [frame(0, 1)], now: 1234)
        XCTAssertEqual(ring.lastPushAt, 1234)
        // Even a burst whose frames are all dropped counts as a push.
        ring.push(epoch: 1, bandCount: bandCount, channels: channels, frames: [SpectrumFrame(pos: 0, bands: [1])], now: 2000)
        XCTAssertEqual(ring.lastPushAt, 2000)
    }

    func testClearDropsFramesAndTheClock() {
        let ring = SpectrumRing()
        push(ring, epoch: 1, [frame(1.0, 10)])
        ring.setAudibleClock(epoch: 1, position: 1.0, now: 100)
        ring.clear()
        XCTAssertNil(ring.pickSpectrumFrame(epoch: nil, target: 1.0, lagSec: 0.25, leadSec: 0.05))
        XCTAssertNil(ring.audibleTarget(now: 100))
    }

    // MARK: - Audible clock

    func testNoTargetBeforeTheFirstTick() {
        XCTAssertNil(SpectrumRing().audibleTarget(now: 1000))
    }

    func testTheTargetExtrapolatesFromTheLastTickPlusTheLead() throws {
        let ring = SpectrumRing()
        ring.setAudibleClock(epoch: 3, position: 10.0, now: 1000)
        let target = try XCTUnwrap(ring.audibleTarget(now: 1100, aheadSec: 0.05))
        XCTAssertEqual(target.epoch, 3)
        XCTAssertEqual(target.pos, 10.0 + 0.1 + 0.05, accuracy: 1e-9)
    }

    func testATickOlderThan250MsIsNoClock() {
        let ring = SpectrumRing()
        ring.setAudibleClock(epoch: 3, position: 10.0, now: 1000)
        XCTAssertNotNil(ring.audibleTarget(now: 1000 + SpectrumRing.audibleClockStaleMs))
        XCTAssertNil(ring.audibleTarget(now: 1000 + SpectrumRing.audibleClockStaleMs + 0.001))
        XCTAssertEqual(SpectrumRing.audibleClockStaleMs, 250)
        // A fresh tick revives it.
        ring.setAudibleClock(epoch: 4, position: 0.1, now: 2000)
        XCTAssertEqual(ring.audibleTarget(now: 2000)?.epoch, 4)
    }

    func testANegativePositionMapsOntoTheEndOfThePreviousEpoch() throws {
        let ring = SpectrumRing()
        // The outgoing track's frames run to 181.5 s; the incoming track's
        // first frames have arrived too.
        push(ring, epoch: 7, [frame(181.4, 1), frame(181.5, 2)])
        push(ring, epoch: 8, [frame(0.0, 3), frame(0.1, 4)])
        // The join is still an audio buffer away: the audible position runs
        // negative on the new track's timeline.
        ring.setAudibleClock(epoch: 8, position: -0.3, now: 1000)
        let target = try XCTUnwrap(ring.audibleTarget(now: 1000))
        XCTAssertEqual(target.epoch, 7)
        XCTAssertEqual(target.pos, 181.2, accuracy: 1e-9)
        // Once the position crosses zero it stays on the new epoch.
        let later = try XCTUnwrap(ring.audibleTarget(now: 1200, aheadSec: 0.15))
        XCTAssertEqual(later.epoch, 8)
        XCTAssertEqual(later.pos, 0.05, accuracy: 1e-9)
    }

    func testANegativePositionStaysOnItsEpochWhenThePreviousOneHasNoFrames() throws {
        let ring = SpectrumRing()
        push(ring, epoch: 5, [frame(0.0, 3)])
        ring.setAudibleClock(epoch: 5, position: -0.2, now: 0)
        let target = try XCTUnwrap(ring.audibleTarget(now: 0))
        XCTAssertEqual(target.epoch, 5)
        XCTAssertEqual(target.pos, -0.2, accuracy: 1e-9)
        // Epoch 0 has no predecessor at all.
        ring.setAudibleClock(epoch: 0, position: -0.2, now: 0)
        XCTAssertEqual(ring.audibleTarget(now: 0)?.epoch, 0)
    }

    func testOnlyTheCurrentAndPreviousEpochsKeepTheirEnd() throws {
        let ring = SpectrumRing()
        push(ring, epoch: 1, [frame(90.0, 1)])
        push(ring, epoch: 2, [frame(60.0, 2)])
        push(ring, epoch: 3, [frame(0.0, 3)])
        // Epoch 1's end was forgotten when epoch 3 arrived; epoch 2's is kept.
        ring.setAudibleClock(epoch: 2, position: -0.5, now: 0)
        XCTAssertEqual(ring.audibleTarget(now: 0)?.epoch, 2)
        ring.setAudibleClock(epoch: 3, position: -0.5, now: 0)
        let target = try XCTUnwrap(ring.audibleTarget(now: 0))
        XCTAssertEqual(target.epoch, 2)
        XCTAssertEqual(target.pos, 59.5, accuracy: 1e-9)
    }

    func testAnEpochsEndIsItsLatestFrameNotItsLastPushed() throws {
        let ring = SpectrumRing()
        push(ring, epoch: 1, [frame(10.0, 1), frame(9.0, 2)])
        push(ring, epoch: 2, [frame(0.0, 3)])
        ring.setAudibleClock(epoch: 2, position: -1.0, now: 0)
        XCTAssertEqual(try XCTUnwrap(ring.audibleTarget(now: 0)).pos, 9.0, accuracy: 1e-9)
    }

    // MARK: - Per-band read-ahead

    /// Frames 1/60 s apart whose every value is its frame index, so a
    /// composite shows which frame each value came from.
    private func steppedRing(epoch: UInt64 = 1, count: Int, start: Int = 0) -> SpectrumRing {
        let ring = SpectrumRing()
        let frames = (start..<(start + count)).map { frame(Double($0) / 60, UInt8($0)) }
        push(ring, epoch: epoch, frames)
        return ring
    }

    func testEachBandIsReadItsOwnNumberOfFramesAhead() {
        let ring = steppedRing(count: 20)
        // Band 0 reads three frames on, band 1 the base frame, on both channels.
        let bands = ring.readSpectrumBands(epoch: 1, target: 5.0 / 60, lagSec: 0.25, leadSec: 0.05, ahead: [3, 0], periodSec: 1.0 / 60)
        XCTAssertEqual(bands, [8, 5, 8, 5])
    }

    func testAStepTheRingDoesNotHoldFallsBackToTheLatestBeforeIt() {
        let ring = steppedRing(count: 7)
        // Frames 0...6 exist; from base 5, steps 1 and 2 want frames 6 and 7.
        let bands = ring.readSpectrumBands(epoch: 1, target: 5.0 / 60, lagSec: 0.25, leadSec: 0.05, ahead: [2, 1], periodSec: 1.0 / 60)
        XCTAssertEqual(bands, [6, 6, 6, 6])
    }

    func testStepsStayInTheBaseFramesEpoch() {
        let ring = steppedRing(epoch: 1, count: 6)
        // The next epoch holds a frame at the position epoch 1's step wants.
        push(ring, epoch: 2, [frame(6.0 / 60, 200)])
        let bands = ring.readSpectrumBands(epoch: 1, target: 5.0 / 60, lagSec: 0.25, leadSec: 0.05, ahead: [1, 0], periodSec: 1.0 / 60)
        XCTAssertEqual(bands, [5, 5, 5, 5])
    }

    func testNothingToStepReturnsTheBaseFrame() {
        let ring = steppedRing(count: 10)
        let read = { (ahead: [UInt8]?, period: Double) in
            ring.readSpectrumBands(epoch: 1, target: 4.0 / 60, lagSec: 0.25, leadSec: 0.05, ahead: ahead, periodSec: period)
        }
        XCTAssertEqual(read(nil, 1.0 / 60), [4, 4, 4, 4])
        XCTAssertEqual(read([], 1.0 / 60), [4, 4, 4, 4])
        XCTAssertEqual(read([0, 0], 1.0 / 60), [4, 4, 4, 4])
        // A step table that doesn't divide the frame, or no frame period.
        XCTAssertEqual(read([1, 1, 1], 1.0 / 60), [4, 4, 4, 4])
        XCTAssertEqual(read([1, 1], 0), [4, 4, 4, 4])
    }

    func testNoFrameNearTheTargetReadsNothing() {
        let ring = steppedRing(count: 10)
        XCTAssertNil(ring.readSpectrumBands(epoch: 1, target: 5, lagSec: 0.25, leadSec: 0.05, ahead: [1, 0], periodSec: 1.0 / 60))
        XCTAssertNil(ring.readSpectrumBands(epoch: 2, target: 4.0 / 60, lagSec: 0.25, leadSec: 0.05, ahead: [1, 0], periodSec: 1.0 / 60))
    }

    // MARK: - Diagnostics

    func testStatsCountFramesAndTheLeadOverTheAudibleClock() throws {
        let ring = SpectrumRing()
        XCTAssertEqual(ring.stats(now: 0).framesPushed, 0)
        XCTAssertNil(ring.stats(now: 0).leadSec)
        push(ring, epoch: 1, [frame(1.0, 1), frame(1.5, 2)])
        ring.setAudibleClock(epoch: 1, position: 1.0, now: 1000)
        let stats = ring.stats(now: 1100)
        XCTAssertEqual(stats.framesPushed, 2)
        XCTAssertEqual(try XCTUnwrap(stats.leadSec), 0.4, accuracy: 1e-9)
        ring.clear()
        XCTAssertEqual(ring.stats(now: 1100).framesPushed, 2)
        XCTAssertNil(ring.stats(now: 1100).leadSec)
    }
}
