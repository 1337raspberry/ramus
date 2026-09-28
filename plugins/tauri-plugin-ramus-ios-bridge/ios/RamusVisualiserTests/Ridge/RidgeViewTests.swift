// Ported from ramusTV `RamusTVTests/Visualiser/RidgeViewTests.swift`.

import XCTest
@testable import RamusVisualiser

/// `RidgeView`: drawable sizing, the first paint, the background rule and
/// the frame-rate switch.
final class RidgeViewTests: XCTestCase {
    private func makeView(width: CGFloat = 192, height: CGFloat = 108) throws -> RidgeView {
        let renderer = try XCTUnwrap(RidgeMetalRenderer(), "Metal is unavailable")
        return RidgeView(frame: CGRect(x: 0, y: 0, width: width, height: height), renderer: renderer)
    }

    func testNothingIsPaintedBeforeStart() throws {
        let view = try makeView()
        view.renderScale = 1
        view.layoutIfNeeded()
        XCTAssertEqual(view.paintsCompleted, 0)
        XCTAssertFalse(view.isRunning)
    }

    func testDrawableFollowsTheRenderScaleInWholePixels() throws {
        let view = try makeView(width: 192.5)
        view.renderScale = 2
        view.start()
        defer { view.stop() }
        view.layoutIfNeeded()
        XCTAssertEqual(view.drawablePixelSize, CGSize(width: 385, height: 216))
        view.renderScale = 1
        view.layoutIfNeeded()
        XCTAssertEqual(view.drawablePixelSize, CGSize(width: 192, height: 108))
    }

    func testFirstLayoutPaintsOneFrame() throws {
        let view = try makeView()
        view.renderScale = 1
        view.start()
        defer { view.stop() }
        view.layoutIfNeeded()
        XCTAssertEqual(view.paintsCompleted, 1)
    }

    func testZeroSizeViewPaintsNothing() throws {
        let view = try makeView(width: 0, height: 0)
        view.start()
        defer { view.stop() }
        view.layoutIfNeeded()
        XCTAssertEqual(view.paintsCompleted, 0)
        XCTAssertEqual(view.drawablePixelSize, .zero)
    }

    func testBackgroundedViewSubmitsNoGPUWork() throws {
        let view = try makeView()
        view.renderScale = 1
        var backgrounded = true
        view.isBackgrounded = { backgrounded }
        view.start()
        defer { view.stop() }
        view.layoutIfNeeded()
        XCTAssertEqual(view.paintsCompleted, 0)
        backgrounded = false
        view.renderScale = 2
        view.layoutIfNeeded()
        XCTAssertEqual(view.paintsCompleted, 1, "paints again in the foreground")
    }

    func testTheFullScreenStackIsDeeper() throws {
        let view = try makeView()
        XCTAssertFalse(view.fullScreen)
        view.fullScreen = true
        XCTAssertTrue(view.fullScreen)
    }

    func testTheFrameRateStartsAtSixtyAndCanChangeWhileRunning() throws {
        let view = try makeView()
        XCTAssertEqual(view.frameRate, .sixty)
        view.start()
        defer { view.stop() }
        view.frameRate = .oneTwenty
        XCTAssertEqual(view.frameRate, .oneTwenty)
    }
}
