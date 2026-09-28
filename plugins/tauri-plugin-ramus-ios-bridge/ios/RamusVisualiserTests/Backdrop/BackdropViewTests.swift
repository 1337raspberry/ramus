// Ported from ramusTV `RamusTVTests/Backdrop/BackdropViewTests.swift`.

import XCTest
@testable import RamusVisualiser

final class BackdropViewTests: XCTestCase {
    var window: UIWindow!
    var view: BackdropView!

    override func setUp() {
        window = UIWindow(frame: UIScreen.main.bounds)
        window.rootViewController = UIViewController()
        window.makeKeyAndVisible()
        view = BackdropView(frame: window.bounds, tone: .sdr, renderer: BackdropRenderer()!)
        window.rootViewController!.view.addSubview(view)
        view.layoutIfNeeded()
    }

    override func tearDown() {
        window.isHidden = true
        window = nil
        // XCTest keeps every test case until the run ends; release the
        // full-screen Metal layer now.
        view = nil
    }

    private func wait(_ seconds: Double) {
        let done = expectation(description: "waited")
        DispatchQueue.main.asyncAfter(deadline: .now() + seconds) { done.fulfill() }
        wait(for: [done], timeout: seconds + 1)
    }

    private let other = BackdropCorners(
        topLeft: BackdropColor(hex: "3348a6")!, topRight: BackdropColor(hex: "a36730")!,
        bottomLeft: BackdropColor(hex: "772b9d")!, bottomRight: BackdropColor(hex: "25978e")!)

    /// The app disables the system's minimum-frame-duration throttle for
    /// the ridge's 120 Hz mode; without its own range this crossfade would
    /// otherwise tick at whatever the display's top rate is too.
    func testTheCrossfadeLinkIsPinnedToSixty() {
        view.setCorners(other, animated: true)
        let range = view.link?.preferredFrameRateRange
        XCTAssertEqual(range?.minimum, 60)
        XCTAssertEqual(range?.maximum, 60)
        XCTAssertEqual(range?.preferred, 60)
    }

    func testItStartsOnTheBrandCorners() {
        XCTAssertEqual(view.targetCorners, .brandDefault)
        XCTAssertTrue(view.isFieldVisible)
    }

    func testTheDrawableIsTheViewInNativePixels() {
        let scale = view.traitCollection.displayScale
        XCTAssertEqual(view.drawablePixelSize.width, (view.bounds.width * scale).rounded(.down))
        XCTAssertEqual(view.drawablePixelSize.height, (view.bounds.height * scale).rounded(.down))
    }

    /// With two drawables, each of a fade's draws waited on the main thread
    /// for the display to hand one back.
    func testTheLayerKeepsThreeDrawables() {
        XCTAssertEqual(view.maximumDrawableCount, 3)
    }

    /// A second change straight after the first can find the first frame
    /// still on the GPU; its draw is skipped and happens on the next frame.
    func testADrawSkippedWhileTheGPUIsBusyHappensOnTheNextFrame() {
        let draws = view.drawsCompleted
        view.setCorners(other, animated: false)
        view.setCorners(.brandDefault, animated: false)
        let drawn = expectation(
            for: NSPredicate { _, _ in !self.view.isDrawPending && self.view.drawsCompleted > draws },
            evaluatedWith: nil)
        wait(for: [drawn], timeout: 2)
    }

    /// Once the field is hidden there is nothing to draw, so a draw left
    /// pending is dropped rather than retried on every frame.
    func testHidingTheFieldDropsAPendingDraw() {
        view.isBackgrounded = { true }
        view.setCorners(other, animated: false)
        XCTAssertTrue(view.isDrawPending)
        view.setShowsColors(false, animated: false)
        XCTAssertFalse(view.isDrawPending)
    }

    func testItDrawsOnceLaidOutInAWindow() {
        XCTAssertGreaterThanOrEqual(view.drawsCompleted, 1)
    }

    /// A new display scale resizes the drawable to the view in the new
    /// scale's pixels and draws it again.
    func testADisplayScaleChangeResizesTheDrawableAndDraws() {
        let native = view.traitCollection.displayScale
        let scale: CGFloat = native == 1 ? 2 : 1
        let draws = view.drawsCompleted
        view.traitOverrides.displayScale = scale
        view.layoutIfNeeded()
        XCTAssertEqual(view.drawablePixelSize.width, (view.bounds.width * scale).rounded(.down))
        XCTAssertEqual(view.drawablePixelSize.height, (view.bounds.height * scale).rounded(.down))
        let drawn = expectation(for: NSPredicate { _, _ in self.view.drawsCompleted > draws }, evaluatedWith: nil)
        wait(for: [drawn], timeout: 2)
    }

    func testBlackHidesTheFieldAndTakesNewColoursWithoutDrawing() {
        view.setShowsColors(false, animated: false)
        XCTAssertFalse(view.showsColors)
        XCTAssertFalse(view.isFieldVisible)
        let draws = view.drawsCompleted
        view.setCorners(other, animated: true)
        XCTAssertEqual(view.targetCorners, other)
        XCTAssertFalse(view.isAnimating, "a hidden field takes new colours without a fade")
        XCTAssertEqual(view.drawsCompleted, draws)
    }

    func testShowingColoursAgainFadesTheFieldIn() {
        view.setShowsColors(false, animated: false)
        let draws = view.drawsCompleted
        view.setShowsColors(true, animated: true)
        XCTAssertTrue(view.isFieldVisible)
        XCTAssertTrue(view.isAnimating)
        wait(BackdropAnimator.strengthSeconds + 0.3)
        XCTAssertFalse(view.isAnimating)
        XCTAssertGreaterThan(view.drawsCompleted, draws + 1)
    }

    func testFadingToBlackHidesTheFieldWhenDone() {
        view.setShowsColors(false, animated: true)
        XCTAssertTrue(view.isFieldVisible)
        wait(BackdropAnimator.strengthSeconds + 0.3)
        XCTAssertFalse(view.isFieldVisible)
    }

    /// In the background no GPU work is submitted; the skipped draw happens
    /// on becoming active again.
    func testADrawInTheBackgroundWaitsForTheForeground() {
        var backgrounded = true
        view.isBackgrounded = { backgrounded }
        let draws = view.drawsCompleted
        view.setCorners(other, animated: false)
        XCTAssertEqual(view.drawsCompleted, draws)
        backgrounded = false
        NotificationCenter.default.post(name: UIApplication.didBecomeActiveNotification, object: nil)
        let drawn = expectation(for: NSPredicate { _, _ in self.view.drawsCompleted == draws + 1 }, evaluatedWith: nil)
        wait(for: [drawn], timeout: 2)
    }
}
