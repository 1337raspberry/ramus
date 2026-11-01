import CoreGraphics
import XCTest
@testable import RamusWidgetCore

final class WaveformPathTests: XCTestCase {
    func testSilhouetteIsClosedAndSymmetric() {
        let rect = CGRect(x: 0, y: 0, width: 100, height: 30)
        let path = WaveformPath.silhouette(levels: [1.0, 1.0, 1.0], in: rect)
        let box = path.boundingBoxOfPath
        // midY 15, maxAmp 13: the shape spans 2...28 vertically, 0...100 across.
        XCTAssertEqual(box.minY, 2, accuracy: 0.001)
        XCTAssertEqual(box.maxY, 28, accuracy: 0.001)
        XCTAssertEqual(box.minX, 0, accuracy: 0.001)
        XCTAssertEqual(box.maxX, 100, accuracy: 0.001)
        XCTAssertTrue(path.contains(CGPoint(x: 50, y: 15)))
    }

    func testSilhouetteOfNothingIsEmpty() {
        XCTAssertTrue(WaveformPath.silhouette(levels: [], in: CGRect(x: 0, y: 0, width: 10, height: 10)).isEmpty)
    }

    func testSeekFractionCentresZones() {
        XCTAssertEqual(WaveformPath.seekFraction(zone: 0, zones: 12), 0.5 / 12, accuracy: 1e-9)
        XCTAssertEqual(WaveformPath.seekFraction(zone: 11, zones: 12), 11.5 / 12, accuracy: 1e-9)
    }
}
