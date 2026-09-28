import XCTest
@testable import RamusVisualiser

/// The frame and clock pushes decode from the JSON Rust sends
/// (`SpectrumFramesPayload` / `PlaybackAudiblePayload`, camelCase).
final class SpectrumPayloadsTests: XCTestCase {
    func testABatchDecodes() throws {
        let json = #"{"epoch":7,"bandCount":2,"channels":2,"frames":[{"pos":1.5,"bands":[1,2,3,4]},{"pos":1.52,"bands":[5,6,7,8]}]}"#
        let batch = try JSONDecoder().decode(NativeSpectrumBatch.self, from: Data(json.utf8))
        XCTAssertEqual(batch.epoch, 7)
        XCTAssertEqual(batch.bandCount, 2)
        XCTAssertEqual(batch.channels, 2)
        XCTAssertEqual(batch.frames, [
            SpectrumFrame(pos: 1.5, bands: [1, 2, 3, 4]),
            SpectrumFrame(pos: 1.52, bands: [5, 6, 7, 8]),
        ])
    }

    func testATickDecodes() throws {
        let tick = try JSONDecoder().decode(NativeAudibleTick.self, from: Data(#"{"epoch":3,"position":-0.12}"#.utf8))
        XCTAssertEqual(tick.epoch, 3)
        XCTAssertEqual(tick.position, -0.12)
    }
}
