import XCTest
@testable import RamusWidgetCore

final class TimelinePlannerTests: XCTestCase {
    // Playing at 50 s of 200 s, read at t = 1000: the track should end at 1150.
    func testPlayingFlipsToPausedAfterTrackEndPlusGrace() {
        let plan = TimelinePlanner.plan(snapshot(snapshotJSON()), now: at(1_010))
        XCTAssertEqual(plan.first, PlannedEntry(date: at(1_010), face: .nowPlaying(paused: false)))
        XCTAssertEqual(plan[1], PlannedEntry(date: at(1_165), face: .nowPlaying(paused: true)))
    }

    func testPlayingGoesIdleAnHourAfterThat() {
        let plan = TimelinePlanner.plan(snapshot(snapshotJSON()), now: at(1_010))
        XCTAssertEqual(plan.count, 3)
        XCTAssertEqual(plan[2], PlannedEntry(date: at(1_165 + 3_600), face: .idle(resume: true)))
    }

    func testZeroDurationPausesNow() {
        let plan = TimelinePlanner.plan(snapshot(snapshotJSON(durationSec: 0)), now: at(1_010))
        XCTAssertEqual(plan, [
            PlannedEntry(date: at(1_010), face: .nowPlaying(paused: true)),
            PlannedEntry(date: at(1_010 + 3_600), face: .idle(resume: true)),
        ])
    }

    func testPositionPastEndPausesNow() {
        let plan = TimelinePlanner.plan(snapshot(snapshotJSON(positionSec: 250)), now: at(1_010))
        XCTAssertEqual(plan.first, PlannedEntry(date: at(1_010), face: .nowPlaying(paused: true)))
        XCTAssertEqual(plan.last, PlannedEntry(date: at(950 + 15 + 3_600), face: .idle(resume: true)))
        // A playing snapshot whose track ended more than an hour ago is idle.
        let stale = TimelinePlanner.plan(snapshot(snapshotJSON()), now: at(5_000))
        XCTAssertEqual(stale, [PlannedEntry(date: at(5_000), face: .idle(resume: true))])
    }

    func testPausedGoesIdleAnHourAfterLastActivity() {
        let plan = TimelinePlanner.plan(snapshot(snapshotJSON(state: "paused", lastActivityAt: 900)), now: at(1_000))
        XCTAssertEqual(plan, [
            PlannedEntry(date: at(1_000), face: .nowPlaying(paused: true)),
            PlannedEntry(date: at(4_500), face: .idle(resume: true)),
        ])
    }

    func testPausedLongAgoIsIdleImmediately() {
        let plan = TimelinePlanner.plan(snapshot(snapshotJSON(state: "paused", lastActivityAt: 0)), now: at(10_000))
        XCTAssertEqual(plan, [PlannedEntry(date: at(10_000), face: .idle(resume: true))])
    }

    func testIdleWithoutResumeWhenNotResumable() {
        let paused = TimelinePlanner.plan(snapshot(snapshotJSON(state: "paused", lastActivityAt: 0, resumable: false)), now: at(10_000))
        XCTAssertEqual(paused, [PlannedEntry(date: at(10_000), face: .idle(resume: false))])
        let idle = TimelinePlanner.plan(snapshot(snapshotJSON(state: "idle", resumable: false)), now: at(3))
        XCTAssertEqual(idle, [PlannedEntry(date: at(3), face: .idle(resume: false))])
        let signedOut = TimelinePlanner.plan(snapshot(snapshotJSON(state: "signedOut", resumable: false)), now: at(3))
        XCTAssertEqual(signedOut, [PlannedEntry(date: at(3), face: .signedOut)])
    }
}
