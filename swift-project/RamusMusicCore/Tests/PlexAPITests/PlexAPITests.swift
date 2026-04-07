import XCTest
@testable import PlexAPI

final class PlexAPITests: XCTestCase {
    func testPlexClientErrorCases() {
        let error = PlexClientError.notConnected
        XCTAssertTrue(error is PlexClientError)
    }
}
