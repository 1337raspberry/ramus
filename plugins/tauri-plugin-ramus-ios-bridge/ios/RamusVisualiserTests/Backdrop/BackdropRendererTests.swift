// Ported from ramusTV `RamusTVTests/Backdrop/BackdropRendererTests.swift`.

import Metal
import XCTest
@testable import RamusVisualiser

/// `BackdropRenderer` drawing offscreen, read back and compared with
/// `BackdropField`'s CPU reference.
final class BackdropRendererTests: XCTestCase {
    private var renderer: BackdropRenderer!
    private var queue: MTLCommandQueue!

    override func setUpWithError() throws {
        renderer = try XCTUnwrap(BackdropRenderer(), "Metal is unavailable")
        queue = try XCTUnwrap(renderer.device.makeCommandQueue())
    }

    /// BGRA bytes, top row first.
    private func render(_ uniforms: BackdropUniforms, width w: Int, height h: Int) throws -> [UInt8] {
        let desc = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: BackdropRenderer.pixelFormat, width: w, height: h, mipmapped: false)
        desc.usage = [.renderTarget]
        desc.storageMode = .private
        let texture = try XCTUnwrap(renderer.device.makeTexture(descriptor: desc))
        let drawn = try XCTUnwrap(renderer.render(uniforms, to: texture))
        drawn.waitUntilCompleted()
        XCTAssertEqual(drawn.status, .completed)
        let buffer = try XCTUnwrap(renderer.device.makeBuffer(length: w * h * 4, options: .storageModeShared))
        let blitBuffer = try XCTUnwrap(queue.makeCommandBuffer())
        let blit = try XCTUnwrap(blitBuffer.makeBlitCommandEncoder())
        blit.copy(
            from: texture, sourceSlice: 0, sourceLevel: 0, sourceOrigin: MTLOrigin(x: 0, y: 0, z: 0),
            sourceSize: MTLSize(width: w, height: h, depth: 1), to: buffer, destinationOffset: 0,
            destinationBytesPerRow: w * 4, destinationBytesPerImage: w * h * 4)
        blit.endEncoding()
        blitBuffer.commit()
        blitBuffer.waitUntilCompleted()
        return Array(UnsafeBufferPointer(start: buffer.contents().assumingMemoryBound(to: UInt8.self), count: w * h * 4))
    }

    /// Four distinct corners (ramus's `four-quadrants` fixture) after the SDR
    /// tone pass.
    private let vivid = BackdropFieldColors(
        BackdropCorners(topLeft: BackdropColor(hex: "3348a6")!, topRight: BackdropColor(hex: "a36730")!,
                        bottomLeft: BackdropColor(hex: "772b9d")!, bottomRight: BackdropColor(hex: "25978e")!),
        tone: .sdr)
    /// Dark, near-neutral corners, where plain rounding bands worst.
    private let dark = BackdropFieldColors(
        BackdropCorners(topLeft: BackdropColor(hex: "282829")!, topRight: BackdropColor(hex: "2b2a2c")!,
                        bottomLeft: BackdropColor(hex: "29282a")!, bottomRight: BackdropColor(hex: "2c2b2b")!),
        tone: .sdr)

    /// Reference value of channel `ch` (0 red, 1 green, 2 blue) at (x, y), in
    /// 8-bit units.
    private func reference(_ colors: BackdropFieldColors, x: Int, y: Int, w: Int, h: Int, ch: Int) -> Double {
        BackdropField.pixel(x: x, y: y, width: w, height: h, colors, tone: .sdr, strength: 1)[ch] * 255
    }

    /// BGRA byte offset of channel `ch` (0 red, 1 green, 2 blue).
    private func offset(_ ch: Int) -> Int { [2, 1, 0][ch] }

    func testEveryPixelIsTheReferenceWithinTheDither() throws {
        let (w, h) = (320, 180)
        for colors in [vivid, dark] {
            let px = try render(BackdropUniforms(colors: colors, tone: .sdr, strength: 1), width: w, height: h)
            var worst = 0.0
            for y in 0..<h {
                for x in 0..<w {
                    for ch in 0..<3 {
                        let got = Double(px[(y * w + x) * 4 + offset(ch)])
                        worst = max(worst, abs(got - reference(colors, x: x, y: y, w: w, h: h, ch: ch)))
                    }
                    XCTAssertEqual(px[(y * w + x) * 4 + 3], 255)
                }
            }
            // The dither spans ±1 step and rounding half a step; the field's
            // bilinear grid adds under 0.1 for these corners.
            XCTAssertLessThanOrEqual(worst, 1.6)
        }
    }

    /// Averaged over 32 × 32 blocks the output follows the float field to a
    /// quarter of a level, so no step between levels shows. Plain rounding of
    /// the same field misses by more, which is the banding the dither removes.
    func testBlockAveragesFollowTheFieldWhereRoundingWouldBand() throws {
        let (w, h, block) = (320, 192, 32)
        let px = try render(BackdropUniforms(colors: dark, tone: .sdr, strength: 1), width: w, height: h)
        var ditheredWorst = 0.0
        var roundedWorst = 0.0
        for by in 0..<(h / block) {
            for bx in 0..<(w / block) {
                for ch in 0..<3 {
                    var got = 0.0, want = 0.0, rounded = 0.0
                    for y in (by * block)..<((by + 1) * block) {
                        for x in (bx * block)..<((bx + 1) * block) {
                            let r = reference(dark, x: x, y: y, w: w, h: h, ch: ch)
                            got += Double(px[(y * w + x) * 4 + offset(ch)])
                            want += r
                            rounded += r.rounded()
                        }
                    }
                    let n = Double(block * block)
                    ditheredWorst = max(ditheredWorst, abs(got - want) / n)
                    roundedWorst = max(roundedWorst, abs(rounded - want) / n)
                }
            }
        }
        XCTAssertLessThanOrEqual(ditheredWorst, 0.25)
        XCTAssertGreaterThan(roundedWorst, 0.3, "the dark field must be one plain rounding bands")
    }

    func testTheDitherIsStatic() throws {
        let u = BackdropUniforms(colors: vivid, tone: .sdr, strength: 1)
        XCTAssertEqual(try render(u, width: 128, height: 72), try render(u, width: 128, height: 72))
    }

    func testTheDitherAveragesOut() throws {
        let (w, h) = (320, 180)
        let px = try render(BackdropUniforms(colors: vivid, tone: .sdr, strength: 1), width: w, height: h)
        var error = 0.0
        for y in 0..<h {
            for x in 0..<w {
                error += Double(px[(y * w + x) * 4 + offset(0)]) - reference(vivid, x: x, y: y, w: w, h: h, ch: 0)
            }
        }
        XCTAssertLessThan(abs(error / Double(w * h)), 0.05)
    }

    /// One frame on the GPU at a time: a draw asked for while the previous
    /// frame is still running is skipped, so the caller never waits.
    func testADrawIsSkippedWhileThePreviousFrameIsOnTheGPU() throws {
        let desc = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: BackdropRenderer.pixelFormat, width: 3840, height: 2160, mipmapped: false)
        desc.usage = [.renderTarget]
        desc.storageMode = .private
        let texture = try XCTUnwrap(renderer.device.makeTexture(descriptor: desc))
        let u = BackdropUniforms(colors: vivid, tone: .sdr, strength: 1)
        let first = try XCTUnwrap(renderer.render(u, to: texture))
        let second = renderer.render(u, to: texture)
        let firstStillRunning = first.status != .completed
        try XCTSkipUnless(firstStillRunning || second == nil, "the first frame finished before the second was asked for")
        XCTAssertNil(second)
        first.waitUntilCompleted()
        XCTAssertNotNil(renderer.render(u, to: texture))
    }

    func testZeroStrengthIsBlack() throws {
        let px = try render(BackdropUniforms(colors: vivid, tone: .sdr, strength: 0), width: 64, height: 36)
        for i in stride(from: 0, to: px.count, by: 4) {
            XCTAssertEqual(Array(px[i..<(i + 4)]), [0, 0, 0, 255])
        }
    }
}
