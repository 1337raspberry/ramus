import XCTest
@testable import RamusVisualiser

final class NativeVisualizerViewTests: XCTestCase {
    private var defaults: UserDefaults!

    override func setUpWithError() throws {
        defaults = try XCTUnwrap(UserDefaults(suiteName: "NativeVisualizerViewTests"))
        defaults.removePersistentDomain(forName: "NativeVisualizerViewTests")
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: "NativeVisualizerViewTests")
    }

    private func showArgs() throws -> NativeVisualizerShowArgs {
        try JSONDecoder().decode(NativeVisualizerShowArgs.self, from: Data(NativeVisualizerArgsTests.showJSON.utf8))
    }

    private func makeView() throws -> (NativeVisualizerView, NativeVisualizerFeed) {
        let args = try showArgs()
        let feed = NativeVisualizerFeed(layout: args.layout, playing: args.playing)
        let renderers = try XCTUnwrap(NativeVisualizerRenderers())
        let view = NativeVisualizerView(args: args, feed: feed, renderers: renderers, defaults: defaults)
        return (view, feed)
    }

    /// The view comes up correctly once its renderers are built separately
    /// and handed in — the shape `showNativeVisualizer` uses, with
    /// `NativeVisualizerRenderers` built off the main thread beforehand.
    func testItComesUpWithSuppliedRenderers() throws {
        let renderers = try XCTUnwrap(NativeVisualizerRenderers())
        let args = try showArgs()
        let feed = NativeVisualizerFeed(layout: args.layout, playing: args.playing)
        let view = NativeVisualizerView(args: args, feed: feed, renderers: renderers, defaults: defaults)
        XCTAssertEqual(view.ridge.params.ridgeAlpha, 0.6)
        XCTAssertTrue(view.ridge.fullScreen)
    }

    func testTheArgsReachTheLayers() throws {
        let (view, _) = try makeView()
        XCTAssertEqual(view.ridge.params.ridgeAlpha, 0.6)
        XCTAssertTrue(view.ridge.fullScreen)
        XCTAssertEqual(view.backdrop.targetCorners, try showArgs().backdrop.corners)
        XCTAssertEqual(view.backdrop.tone.opacity, 0.95)
    }

    func testItIsTheCloseButtonForAccessibility() throws {
        let (view, _) = try makeView()
        XCTAssertTrue(view.isAccessibilityElement)
        XCTAssertEqual(view.accessibilityLabel, "Close visualiser")
        XCTAssertTrue(view.accessibilityTraits.contains(.button))
        XCTAssertTrue(view.accessibilityViewIsModal, "VoiceOver must not reach the covered web page")
    }

    func testATapDismisses() throws {
        let (view, _) = try makeView()
        var dismissed = 0
        view.onDismiss = { dismissed += 1 }
        view.handleTap()
        XCTAssertEqual(dismissed, 1)
        XCTAssertTrue(view.accessibilityActivate())
        XCTAssertEqual(dismissed, 2)
    }

    func testALongPressSwitchesTheRateAndRemembersIt() throws {
        let (view, _) = try makeView()
        XCTAssertEqual(view.ridge.frameRate, .sixty)
        view.toggleFrameRate()
        XCTAssertEqual(view.ridge.frameRate, .oneTwenty)
        XCTAssertEqual(RidgeFrameRate.load(defaults), .oneTwenty)
        let (again, _) = try makeView()
        XCTAssertEqual(again.ridge.frameRate, .oneTwenty, "a new screen opens at the stored rate")
    }

    func testNewColoursCrossfade() throws {
        let (view, _) = try makeView()
        let next = try JSONDecoder().decode(
            NativeBackdrop.self,
            from: Data(#"{"topLeft":[1,2,3],"topRight":[4,5,6],"bottomLeft":[7,8,9],"bottomRight":[10,11,12],"opacity":0.95}"#.utf8))
        view.setBackdrop(next)
        XCTAssertEqual(view.backdrop.targetCorners, next.corners)
    }

    func testTheFeedDrivesTheReader() throws {
        let (_, feed) = try makeView()
        let frames = (0..<30).map { k in SpectrumFrame(pos: Double(k) / 60, bands: [UInt8](repeating: UInt8(k), count: 128)) }
        feed.push(NativeSpectrumBatch(epoch: 2, bandCount: 64, channels: 2, frames: frames), now: 0)
        feed.setAudible(NativeAudibleTick(epoch: 2, position: 10.0 / 60), now: 0)
        XCTAssertNotNil(feed.reader.bands(now: 0, leadSec: 0))
        feed.isPlaying = false
        XCTAssertNil(feed.reader.bands(now: 0, leadSec: 0))
        feed.isPlaying = true
        feed.clearFrames()
        XCTAssertNil(feed.reader.bands(now: 0, leadSec: 0))
    }
}
