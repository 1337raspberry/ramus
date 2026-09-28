// Ported from ramusTV `RamusTVTests/Backdrop/BackdropFieldTests.swift`.

import XCTest
@testable import RamusVisualiser

final class BackdropFieldTests: XCTestCase {
    func testConstantsMatchRamus() throws {
        let golden = try BackdropGolden.load()
        XCTAssertEqual([BackdropField.base.x, BackdropField.base.y, BackdropField.base.z], golden.fieldBase)
        XCTAssertEqual(BackdropField.falloffStops.map { [$0.0, $0.1] }, golden.falloffStops)
        XCTAssertEqual(BackdropField.transitionSeconds * 1000, golden.transitionMs, accuracy: 1e-9)
        XCTAssertEqual(BackdropField.page, 17)
    }

    func testEaseInOutMatchesRamus() throws {
        for sample in try BackdropGolden.load().easeInOut {
            XCTAssertEqual(BackdropField.easeInOut(sample[0]), sample[1], accuracy: 1e-12, "x = \(sample[0])")
        }
    }

    func testFalloffPassesThroughItsStops() {
        XCTAssertEqual(BackdropField.falloff(0), 1, accuracy: 1e-12)
        XCTAssertEqual(BackdropField.falloff(0.2), 0.775, accuracy: 1e-12)
        XCTAssertEqual(BackdropField.falloff(0.4), 0.55, accuracy: 1e-12)
        XCTAssertEqual(BackdropField.falloff(0.62), 0.18, accuracy: 1e-12)
        XCTAssertEqual(BackdropField.falloff(0.8), 0, accuracy: 1e-12)
        XCTAssertEqual(BackdropField.falloff(1.2), 0)
    }

    /// ramus `adjustedRgb`, SDR (saturation 1.3) and HDR (saturation 1.05,
    /// brightness 0.9); expected values from ramus's formula run in Node.
    func testTonePassMatchesRamus() {
        let cases: [(String, [UInt8], [UInt8])] = [
            ("853e43", [147, 54, 61], [122, 55, 59]),
            ("875147", [147, 77, 64], [123, 72, 63]),
            ("853646", [147, 45, 65], [122, 47, 62]),
            ("854256", [144, 57, 83], [121, 58, 77]),
            ("ff0000", [255, 0, 0], [237, 0, 0]),
            ("282829", [40, 40, 41], [36, 36, 37]),
            ("8a7f17", [151, 136, 1], [126, 116, 17]),
        ]
        for (hex, sdr, hdr) in cases {
            let c = BackdropColor(hex: hex)!
            let s = BackdropTone.sdr.adjusted(c)
            let h = BackdropTone.hdr.adjusted(c)
            XCTAssertEqual([s.red, s.green, s.blue], sdr, "sdr \(hex)")
            XCTAssertEqual([h.red, h.green, h.blue], hdr, "hdr \(hex)")
        }
    }

    private let colors = BackdropFieldColors(
        topLeft: SIMD3(200, 40, 40), topRight: SIMD3(40, 200, 40),
        bottomLeft: SIMD3(40, 40, 200), bottomRight: SIMD3(200, 200, 40))

    /// Top-left paints last, so it is exact at its own corner.
    func testTopLeftIsExactAtItsCorner() {
        let c = BackdropField.color(at: SIMD2(0, 0), colors)
        XCTAssertEqual(c.x * 255, 200, accuracy: 1e-9)
        XCTAssertEqual(c.y * 255, 40, accuracy: 1e-9)
        XCTAssertEqual(c.z * 255, 40, accuracy: 1e-9)
    }

    /// At the bottom-right corner, top-right (one side's length away, so at
    /// √½ of its ray) paints over bottom-right with the coverage it has
    /// left; top-left, a whole ray away, adds nothing.
    func testLaterLayersPaintOverEarlierOnes() {
        let a = BackdropField.falloff(0.70710678)
        let expected = colors.topRight * a + colors.bottomRight * (1 - a)
        let c = BackdropField.color(at: SIMD2(1, 1), colors) * 255
        XCTAssertEqual(c.x, expected.x, accuracy: 1e-6)
        XCTAssertEqual(c.y, expected.y, accuracy: 1e-6)
        XCTAssertEqual(c.z, expected.z, accuracy: 1e-6)
    }

    func testPixelsAreDimmedOverThePageAndScaledByStrength() {
        let flat = BackdropFieldColors(topLeft: SIMD3(repeating: 100), topRight: SIMD3(repeating: 100),
                                       bottomLeft: SIMD3(repeating: 100), bottomRight: SIMD3(repeating: 100))
        let field = BackdropField.color(at: SIMD2(0.5 / 10, 0.5 / 10), flat)
        let full = BackdropField.pixel(x: 0, y: 0, width: 10, height: 10, flat, tone: .sdr, strength: 1)
        XCTAssertEqual(full.x, field.x * 0.95 + 17.0 / 255 * 0.05, accuracy: 1e-12)
        let half = BackdropField.pixel(x: 0, y: 0, width: 10, height: 10, flat, tone: .sdr, strength: 0.5)
        XCTAssertEqual(half.x, full.x * 0.5, accuracy: 1e-12)
        XCTAssertEqual(BackdropField.pixel(x: 3, y: 7, width: 10, height: 10, flat, tone: .sdr, strength: 0), .zero)
    }

    func testFieldColorsComeFromTheTonePass() {
        let colors = BackdropFieldColors(.brandDefault, tone: .sdr)
        XCTAssertEqual(colors.topLeft, SIMD3(147, 54, 61))
        XCTAssertEqual(colors.bottomRight, SIMD3(144, 57, 83))
    }

    func testMixedInterpolatesFromTheFirst() {
        let a = BackdropFieldColors(topLeft: SIMD3(repeating: 0), topRight: SIMD3(repeating: 10),
                                    bottomLeft: SIMD3(repeating: 20), bottomRight: SIMD3(repeating: 30))
        let b = BackdropFieldColors(topLeft: SIMD3(repeating: 100), topRight: SIMD3(repeating: 10),
                                    bottomLeft: SIMD3(repeating: 0), bottomRight: SIMD3(repeating: 30))
        let m = a.mixed(to: b, 0.25)
        XCTAssertEqual(m.topLeft, SIMD3(repeating: 25))
        XCTAssertEqual(m.bottomLeft, SIMD3(repeating: 15))
        XCTAssertEqual(a.mixed(to: b, 1), b)
    }

    /// Must match `BackdropUniforms` in `backdropShaderSource` (`Shaders/BackdropShaderSource.swift`).
    func testUniformsLayoutMatchesTheShader() {
        XCTAssertEqual(MemoryLayout<BackdropUniforms>.stride, 96)
        XCTAssertEqual(MemoryLayout<BackdropUniforms>.offset(of: \.bottomLeft), 0)
        XCTAssertEqual(MemoryLayout<BackdropUniforms>.offset(of: \.bottomRight), 16)
        XCTAssertEqual(MemoryLayout<BackdropUniforms>.offset(of: \.topRight), 32)
        XCTAssertEqual(MemoryLayout<BackdropUniforms>.offset(of: \.topLeft), 48)
        XCTAssertEqual(MemoryLayout<BackdropUniforms>.offset(of: \.base), 64)
        XCTAssertEqual(MemoryLayout<BackdropUniforms>.offset(of: \.viewport), 80)
        XCTAssertEqual(MemoryLayout<BackdropUniforms>.offset(of: \.opacity), 88)
        XCTAssertEqual(MemoryLayout<BackdropUniforms>.offset(of: \.strength), 92)
    }

    func testUniformsCarryTheColoursToneAndStrength() {
        let u = BackdropUniforms(colors: colors, tone: .hdr, strength: 0.25)
        XCTAssertEqual(u.topLeft, SIMD4(200 / 255, 40 / 255, 40 / 255, 0))
        XCTAssertEqual(u.bottomLeft, SIMD4(40 / 255, 40 / 255, 200 / 255, 0))
        XCTAssertEqual(u.base, SIMD4(5 / 255, 5 / 255, 8 / 255, 17 / 255))
        XCTAssertEqual(u.opacity, 0.8)
        XCTAssertEqual(u.strength, 0.25)
        XCTAssertEqual(u.viewport, .zero)
    }
}
