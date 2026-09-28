import XCTest
@testable import RamusVisualiser

final class RidgeFrameRateTests: XCTestCase {
    private var defaults: UserDefaults!

    override func setUpWithError() throws {
        defaults = try XCTUnwrap(UserDefaults(suiteName: "RidgeFrameRateTests"))
        defaults.removePersistentDomain(forName: "RidgeFrameRateTests")
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: "RidgeFrameRateTests")
    }

    func testSixtyIsTheDefault() {
        XCTAssertEqual(RidgeFrameRate.load(defaults), .sixty)
    }

    func testTheChoiceIsRemembered() {
        RidgeFrameRate.oneTwenty.save(defaults)
        XCTAssertEqual(RidgeFrameRate.load(defaults), .oneTwenty)
        XCTAssertEqual(defaults.integer(forKey: RidgeFrameRate.defaultsKey), 120)
    }

    func testAnUnknownStoredRateLoadsAsSixty() {
        defaults.set(90, forKey: RidgeFrameRate.defaultsKey)
        XCTAssertEqual(RidgeFrameRate.load(defaults), .sixty)
    }

    func testToggling() {
        XCTAssertEqual(RidgeFrameRate.sixty.toggled, .oneTwenty)
        XCTAssertEqual(RidgeFrameRate.oneTwenty.toggled, .sixty)
    }

    func testDisplayLinkRanges() {
        XCTAssertEqual(RidgeFrameRate.sixty.range, CAFrameRateRange(minimum: 60, maximum: 60, preferred: 60))
        XCTAssertEqual(RidgeFrameRate.oneTwenty.range, CAFrameRateRange(minimum: 80, maximum: 120, preferred: 120))
    }
}
