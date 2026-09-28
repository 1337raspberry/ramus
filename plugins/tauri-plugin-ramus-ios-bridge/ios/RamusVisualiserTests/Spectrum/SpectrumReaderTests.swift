// Ported from ramusTV `RamusTVTests/Tap/SpectrumReaderTests.swift`.

import XCTest
@testable import RamusVisualiser

/// The paint loop's ring read, ported from the ridge path of
/// `ui/src/components/FocusVisualizer.tsx`.
final class SpectrumReaderTests: XCTestCase {
    private let period = 1.0 / 60

    /// A reader over a two-band, two-channel ring with the given step table.
    private func makeReader(ahead: [UInt8] = [0, 0]) -> (SpectrumReader, SpectrumRing) {
        let ring = SpectrumRing()
        let reader = SpectrumReader(ring: ring, bandAhead: ahead, framePeriodS: period)
        reader.isPlayingProvider = { true }
        return (reader, ring)
    }

    /// Frames 1/60 s apart from `start`, each value its frame index plus `offset`.
    private func pushSteps(_ ring: SpectrumRing, epoch: UInt64, count: Int, offset: UInt8 = 0) {
        let frames = (0..<count).map { k in
            SpectrumFrame(pos: Double(k) * period, bands: [UInt8](repeating: UInt8(k) + offset, count: 4))
        }
        ring.push(epoch: epoch, bandCount: 2, channels: 2, frames: frames, now: 0)
    }

    func testToleranceConstantsMatchTheVisualiser() {
        XCTAssertEqual(SpectrumReader.frameLagToleranceS, 0.25)
        XCTAssertEqual(SpectrumReader.frameLeadToleranceS, 0.05)
    }

    func testBandAheadComesFromTheOnsetDelays() {
        let delays = [0.048, 0.03, 0.009, 0.001]
        XCTAssertEqual(SpectrumReader.bandAhead(onsetDelays: delays, framePeriodS: period), [3, 2, 1, 0])
        XCTAssertEqual(SpectrumReader.bandAhead(onsetDelays: [.nan, -1, 100], framePeriodS: period), [0, 0, 255])
    }

    /// An onset table that doesn't divide the frame width reads the base
    /// frame rather than indexing past it.
    func testAnAheadTableThatDoesNotFitReadsTheBaseFrame() {
        let (reader, ring) = makeReader(ahead: [2, 0, 1])
        pushSteps(ring, epoch: 1, count: 30)
        ring.setAudibleClock(epoch: 1, position: 10 * period, now: 0)
        XCTAssertEqual(reader.bands(now: 0, leadSec: 0), [10, 10, 10, 10])
    }

    func testNothingIsReadWhilePlaybackIsNotPlaying() {
        let (reader, ring) = makeReader()
        pushSteps(ring, epoch: 1, count: 30)
        ring.setAudibleClock(epoch: 1, position: 0.2, now: 1000)
        reader.isPlayingProvider = { false }
        XCTAssertFalse(reader.isPlaying)
        XCTAssertNil(reader.bands(now: 1000, leadSec: 0))
        reader.isPlayingProvider = { true }
        XCTAssertTrue(reader.isPlaying)
        XCTAssertNotNil(reader.bands(now: 1000, leadSec: 0))
    }

    func testReadsTheAudibleEpochAtTheLead() {
        let (reader, ring) = makeReader()
        pushSteps(ring, epoch: 1, count: 30, offset: 100)
        pushSteps(ring, epoch: 2, count: 30)
        ring.setAudibleClock(epoch: 2, position: 5 * period, now: 1000)
        // 50 ms on the wall clock plus a 50 ms lead: six frames further on.
        XCTAssertEqual(reader.bands(now: 1050, leadSec: 0.05), [11, 11, 11, 11])
    }

    func testNoTickReadsNothing() {
        let (reader, ring) = makeReader()
        pushSteps(ring, epoch: 1, count: 30)
        XCTAssertNil(reader.bands(now: 0, leadSec: 0))
    }

    func testAStaleTickReadsNothing() {
        let (reader, ring) = makeReader()
        pushSteps(ring, epoch: 1, count: 30)
        ring.setAudibleClock(epoch: 1, position: 2 * period, now: 1000)
        XCTAssertNil(reader.bands(now: 1300, leadSec: 0))
    }

    func testAFrameOutsideTheTolerancesIsNotDrawn() {
        let (reader, ring) = makeReader()
        pushSteps(ring, epoch: 1, count: 1)
        // The only frame is at 0 s: 0.3 s behind the target is too far.
        ring.setAudibleClock(epoch: 1, position: 0.3, now: 0)
        XCTAssertNil(reader.bands(now: 0, leadSec: 0))
        // 0.2 s behind is drawn.
        ring.setAudibleClock(epoch: 1, position: 0.2, now: 0)
        XCTAssertEqual(reader.bands(now: 0, leadSec: 0), [0, 0, 0, 0])
    }

    func testEachBandIsReadAheadByItsStepCount() {
        let (reader, ring) = makeReader(ahead: [2, 0])
        pushSteps(ring, epoch: 1, count: 30)
        ring.setAudibleClock(epoch: 1, position: 10 * period, now: 0)
        XCTAssertEqual(reader.bands(now: 0, leadSec: 0), [12, 10, 12, 10])
    }
}
