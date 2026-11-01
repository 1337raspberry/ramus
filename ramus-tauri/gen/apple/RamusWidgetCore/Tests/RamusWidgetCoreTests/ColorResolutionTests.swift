import CoreGraphics
import XCTest
@testable import RamusWidgetCore

final class ColorResolutionTests: XCTestCase {
    private func colors(source: String?) -> WidgetSnapshot.Colors {
        WidgetSnapshot.Colors(
            topLeft: "a02d32", topRight: "a02d32", bottomRight: "a02d32", bottomLeft: "a02d32",
            accent: "3ddb7a", source: source
        )
    }

    /// A flat 128×128 cover whose extracted corners are all #a02d32.
    private func redCover() -> CGImage {
        let side = CornerColors.intermediateSize
        var data = [UInt8](repeating: 255, count: side * side * 4)
        for i in stride(from: 0, to: data.count, by: 4) {
            data[i] = 170; data[i + 1] = 40; data[i + 2] = 45
        }
        return data.withUnsafeMutableBytes { buf in
            CGContext(
                data: buf.baseAddress, width: side, height: side, bitsPerComponent: 8,
                bytesPerRow: side * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            )!.makeImage()!
        }
    }

    func testSavedArtCornersAreOnlyToned() {
        let r = colors(source: "art").resolved(cover: nil)
        XCTAssertEqual(r.topLeft, "782125")
        XCTAssertEqual(r.accent, "3ddb7a")
        XCTAssertEqual(r.source, "art")
    }

    func testServerCornersAreReplacedFromTheCover() {
        let server = WidgetSnapshot.Colors(
            topLeft: "1a1e45", topRight: "2d572f", bottomRight: "4f6731", bottomLeft: "11323e",
            accent: "3ddb7a", source: "server"
        )
        let r = server.resolved(cover: redCover())
        XCTAssertEqual(r.source, "art")
        XCTAssertEqual([r.topLeft, r.topRight, r.bottomRight, r.bottomLeft], Array(repeating: "782125", count: 4))
    }

    func testServerCornersWithoutACoverAreToned() {
        let r = colors(source: "server").resolved(cover: nil)
        XCTAssertEqual(r.source, "server")
        XCTAssertEqual(r.topLeft, "782125")
    }

    func testTheBrandGradientIsLeftAlone() {
        let r = colors(source: "default").resolved(cover: nil)
        XCTAssertEqual(r.topLeft, "a02d32")
    }

    func testASnapshotWithoutASourceDecodes() {
        let s = snapshot(snapshotJSON())
        XCTAssertNil(s.colors?.source)
    }
}
