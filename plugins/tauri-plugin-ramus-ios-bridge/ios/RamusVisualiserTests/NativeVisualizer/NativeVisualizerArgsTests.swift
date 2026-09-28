import XCTest
@testable import RamusVisualiser

/// The show and update commands decode from the JSON Rust sends.
final class NativeVisualizerArgsTests: XCTestCase {
    static let showJSON = """
    {"params":{"ridgeRows":51,"ridgeFullScreenRows":67,"ridgeAlpha":0.6,"barAlpha":0.3},
     "backdrop":{"topLeft":[10,20,30],"topRight":[40,50,60],"bottomLeft":[70,80,90],"bottomRight":[100,110,120],"opacity":0.95},
     "playing":true,
     "layout":{"bandCount":64,"fps":60,"onsetDelays":[0.048,0.001]}}
    """

    func testShowArgsDecode() throws {
        let args = try JSONDecoder().decode(NativeVisualizerShowArgs.self, from: Data(Self.showJSON.utf8))
        XCTAssertEqual(args.params.ridgeFullScreenRows, 67)
        XCTAssertEqual(args.params.ridgeAlpha, 0.6)
        XCTAssertTrue(args.playing)
        XCTAssertEqual(args.layout, NativeSpectrumLayout(bandCount: 64, fps: 60, onsetDelays: [0.048, 0.001]))
        XCTAssertEqual(args.backdrop.corners, BackdropCorners(
            topLeft: BackdropColor(red: 10, green: 20, blue: 30),
            topRight: BackdropColor(red: 40, green: 50, blue: 60),
            bottomLeft: BackdropColor(red: 70, green: 80, blue: 90),
            bottomRight: BackdropColor(red: 100, green: 110, blue: 120)))
        XCTAssertEqual(args.backdrop.tone, BackdropTone(saturation: 1, brightness: 1, opacity: 0.95))
    }

    func testUpdateArgsDecodeWithAnyFieldMissing() throws {
        let only = try JSONDecoder().decode(NativeVisualizerUpdateArgs.self, from: Data(#"{"clearFrames":true}"#.utf8))
        XCTAssertNil(only.backdrop)
        XCTAssertNil(only.playing)
        XCTAssertEqual(only.clearFrames, true)
        let playing = try JSONDecoder().decode(NativeVisualizerUpdateArgs.self, from: Data(#"{"playing":false}"#.utf8))
        XCTAssertEqual(playing.playing, false)
    }

    func testABackdropWithoutThreeChannelsIsIgnored() throws {
        let json = #"{"topLeft":[1,2],"topRight":[1,2,3],"bottomLeft":[1,2,3],"bottomRight":[1,2,3],"opacity":3}"#
        let backdrop = try JSONDecoder().decode(NativeBackdrop.self, from: Data(json.utf8))
        XCTAssertNil(backdrop.corners)
        XCTAssertEqual(backdrop.tone.opacity, 1, "opacity is clamped to 0...1")
    }

    /// The tone pass is the identity: the frontend sends colours it has
    /// already toned (`adjustedRgb`), so only the dim remains.
    func testTheToneKeepsTheColoursAsSent() throws {
        let backdrop = try JSONDecoder().decode(
            NativeBackdrop.self,
            from: Data(#"{"topLeft":[200,40,40],"topRight":[1,2,3],"bottomLeft":[4,5,6],"bottomRight":[7,8,9],"opacity":0.8}"#.utf8))
        let corners = try XCTUnwrap(backdrop.corners)
        XCTAssertEqual(backdrop.tone.adjusted(corners.topLeft), corners.topLeft)
    }
}
