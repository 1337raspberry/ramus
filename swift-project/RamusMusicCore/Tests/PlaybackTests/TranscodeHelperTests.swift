import XCTest
import Foundation
@testable import Playback
@testable import Models

final class TranscodeHelperTests: XCTestCase {

    // MARK: - URL Building

    func testDirectPlayURLIncludesToken() {
        let server = URL(string: "http://192.168.1.100:32400")!
        let url = TranscodeHelper.buildDirectPlayURL(
            serverURL: server, partKey: "/library/parts/12345/file.flac", token: "abc123"
        )
        XCTAssertNotNil(url)
        let urlString = url!.absoluteString
        XCTAssertTrue(urlString.contains("X-Plex-Token=abc123"))
        XCTAssertTrue(urlString.contains("/library/parts/12345/file.flac"))
    }

    func testHLSURLHasRequiredParameters() {
        let server = URL(string: "http://192.168.1.100:32400")!
        let url = TranscodeHelper.buildHLSURL(
            serverURL: server, token: "abc123",
            trackRatingKey: "9876", clientIdentifier: "test-client-id",
            session: "fixed-session"
        )
        XCTAssertNotNil(url)
        let urlString = url!.absoluteString
        XCTAssertTrue(urlString.contains("/music/:/transcode/universal/start.m3u8?"))
        XCTAssertTrue(urlString.contains("X-Plex-Token=abc123"))
        XCTAssertTrue(urlString.contains("path=/library/metadata/9876"))
        XCTAssertTrue(urlString.contains("X-Plex-Platform=Chrome"))
        XCTAssertTrue(urlString.contains("protocol=hls"))
    }

    func testHLSURLContainsClientProfile() {
        let server = URL(string: "http://192.168.1.100:32400")!
        let url = TranscodeHelper.buildHLSURL(
            serverURL: server, token: "abc123",
            trackRatingKey: "9876", clientIdentifier: "test-client-id"
        )
        XCTAssertNotNil(url)
        let urlString = url!.absoluteString
        XCTAssertTrue(urlString.contains("X-Plex-Client-Profile-Extra="))
        XCTAssertTrue(urlString.contains("musicProfile"))
        XCTAssertTrue(urlString.contains("mpegts"))
    }

    func testHLSURLFixedBitrate() {
        let server = URL(string: "http://192.168.1.100:32400")!
        let url = TranscodeHelper.buildHLSURL(
            serverURL: server, token: "abc123",
            trackRatingKey: "9876", clientIdentifier: "test-client-id"
        )
        XCTAssertNotNil(url)
        let urlString = url!.absoluteString
        XCTAssertTrue(urlString.contains("maxAudioBitrate=256"))
        XCTAssertTrue(urlString.contains("directPlay=0"))
        XCTAssertTrue(urlString.contains("directStreamAudio=0"))
    }
}
