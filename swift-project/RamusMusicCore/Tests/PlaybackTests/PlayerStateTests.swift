import XCTest
import Foundation
@testable import Models

final class PlayerStateTests: XCTestCase {

    func testDefaultStateIsStopped() {
        let state = PlayerState()
        XCTAssertEqual(state.status, .stopped)
        XCTAssertNil(state.currentTrack)
        XCTAssertTrue(state.queue.isEmpty)
        XCTAssertEqual(state.queueIndex, 0)
    }

    func testTrackIsIdentifiableByRatingKey() {
        let track = Track(ratingKey: "abc", title: "T", artistName: "A", albumTitle: "Al")
        XCTAssertEqual(track.id, "abc")
    }
}
