import XCTest
@testable import Models

final class TrackTests: XCTestCase {

    private func makeTrack(artistName: String = "Album Artist", trackArtist: String? = nil) -> Track {
        Track(
            ratingKey: "1", title: "Song", artistName: artistName,
            trackArtist: trackArtist, albumTitle: "Album"
        )
    }

    // MARK: - hasTrackArtist

    func testNilTrackArtistReturnsFalse() {
        let track = makeTrack(trackArtist: nil)
        XCTAssertFalse(track.hasTrackArtist)
    }

    func testEmptyTrackArtistReturnsFalse() {
        let track = makeTrack(trackArtist: "")
        XCTAssertFalse(track.hasTrackArtist)
    }

    func testSameAsAlbumArtistReturnsFalse() {
        let track = makeTrack(artistName: "Radiohead", trackArtist: "Radiohead")
        XCTAssertFalse(track.hasTrackArtist)
    }

    func testSameAsAlbumArtistCaseInsensitiveReturnsFalse() {
        let track = makeTrack(artistName: "Radiohead", trackArtist: "radiohead")
        XCTAssertFalse(track.hasTrackArtist)
    }

    func testDifferentTrackArtistReturnsTrue() {
        let track = makeTrack(artistName: "Various Artists", trackArtist: "Radiohead")
        XCTAssertTrue(track.hasTrackArtist)
    }

    // MARK: - displayArtist

    func testDisplayArtistNilTrackArtist() {
        let track = makeTrack(artistName: "Radiohead", trackArtist: nil)
        XCTAssertEqual(track.displayArtist, "Radiohead")
    }

    func testDisplayArtistEmptyTrackArtist() {
        let track = makeTrack(artistName: "Radiohead", trackArtist: "")
        XCTAssertEqual(track.displayArtist, "Radiohead")
    }

    func testDisplayArtistWithOverride() {
        let track = makeTrack(artistName: "Various Artists", trackArtist: "Thom Yorke")
        XCTAssertEqual(track.displayArtist, "Thom Yorke")
    }
}
