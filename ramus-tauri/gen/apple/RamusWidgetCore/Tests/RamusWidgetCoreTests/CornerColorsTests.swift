import CoreGraphics
import XCTest
@testable import RamusWidgetCore

/// Expected corners come from running the app's own extraction
/// (`ui/src/lib/blurArt.ts`) over the same 128×128 pixels.
final class CornerColorsTests: XCTestCase {
    private static let size = CornerColors.intermediateSize

    private func pixels(_ f: (Int, Int) -> (Int, Int, Int)) -> [UInt8] {
        var out = [UInt8](repeating: 255, count: Self.size * Self.size * 4)
        for y in 0..<Self.size {
            for x in 0..<Self.size {
                let (r, g, b) = f(x, y)
                let i = (y * Self.size + x) * 4
                out[i] = UInt8(r)
                out[i + 1] = UInt8(g)
                out[i + 2] = UInt8(b)
            }
        }
        return out
    }

    private func corners(_ f: (Int, Int) -> (Int, Int, Int)) -> [String] {
        let c = CornerColors.fromPixels(pixels(f))
        return [c.topLeft, c.topRight, c.bottomRight, c.bottomLeft]
    }

    func testAFeatureInOneCornerKeepsItsHue() {
        let c = corners { x, y in x >= 96 && y < 32 ? (40, 200, 70) : (170, 40, 45) }
        XCTAssertEqual(c, ["a02d32", "459d3e", "a02d32", "a02d32"])
    }

    func testAGradientSplitsAcrossCorners() {
        let c = corners { x, y in
            let fx = Double(x) / 127, fy = Double(y) / 127
            return (Int((fx * 230).rounded()), Int((fx * 210).rounded()), Int((220 - fx * 200 - fy * 20).rounded()))
        }
        XCTAssertEqual(c, ["302ea0", "8b7f20", "8c801a", "2d2a9d"])
    }

    func testNearBlackLiftsToCharcoal() {
        let c = corners { x, y in (8 + (x + y) % 3, 8 + (x * y) % 3, 9) }
        XCTAssertEqual(c, ["282828", "282828", "282828", "282828"])
    }

    func testAPaleSkyIsPulledDown() {
        let c = corners { _, y in y < 64 ? (215, 225, 240) : (30, 34, 28) }
        XCTAssertEqual(c, ["737981", "737981", "262a24", "262a24"])
    }

    func testSaturatedQuadrantsAreCapped() {
        let c = corners { x, y in
            x < 64 ? (y < 64 ? (250, 20, 20) : (20, 20, 250)) : (y < 64 ? (250, 240, 10) : (20, 230, 30))
        }
        XCTAssertEqual(c, ["9e2b2b", "847f11", "25982b", "2b2b9e"])
    }

    func testAnImageIsDownscaledBeforeExtraction() throws {
        // A flat 512×512 red: any resampling leaves it flat, so the result
        // must equal the 128×128 extraction of the same colour.
        let side = 512
        var data = [UInt8](repeating: 255, count: side * side * 4)
        for i in stride(from: 0, to: data.count, by: 4) {
            data[i] = 170; data[i + 1] = 40; data[i + 2] = 45
        }
        let image = try XCTUnwrap(data.withUnsafeMutableBytes { buf -> CGImage? in
            let ctx = CGContext(
                data: buf.baseAddress, width: side, height: side, bitsPerComponent: 8,
                bytesPerRow: side * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            )
            return ctx?.makeImage()
        })
        let fromImage = try XCTUnwrap(CornerColors.fromImage(image))
        XCTAssertEqual(fromImage, CornerColors.fromPixels(pixels { _, _ in (170, 40, 45) }))
    }
}

final class BackdropToneTests: XCTestCase {
    func testToneMatchesTheAppsBackdropOnAnHDRPhone() {
        // Saturation ×1.05, brightness ×0.9, then 80 % over the sheet's #0a0a0c.
        XCTAssertEqual(BackdropTone.display("a02d32"), "782125")
        XCTAssertEqual(BackdropTone.display("#282828"), "1f1f1f")
    }
}
