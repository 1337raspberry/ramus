// Ported from ramusTV `RamusTVTests/Visualiser/RidgeMetalTests.swift`.

import Metal
import QuartzCore
import XCTest
@testable import RamusVisualiser

/// `RidgeMetalRenderer` drawing offscreen, read back pixel by pixel. Each
/// check pins one drawing rule: the pixel grid, butt caps, the
/// erase, the fade and single-draw joins.
final class RidgeMetalTests: XCTestCase {
    private var renderer: RidgeMetalRenderer!
    private var queue: MTLCommandQueue!

    override func setUpWithError() throws {
        renderer = try XCTUnwrap(RidgeMetalRenderer(), "Metal is unavailable")
        queue = try XCTUnwrap(renderer.device.makeCommandQueue())
    }

    /// Premultiplied BGRA pixels, top row first.
    struct Pixels {
        let width: Int
        let height: Int
        let bytes: [UInt8]

        func alpha(x: Int, y: Int) -> UInt8 {
            guard x >= 0, x < width, y >= 0, y < height else { return 0 }
            return bytes[(y * width + x) * 4 + 3]
        }
    }

    private func render(_ frame: RidgeFrame?, _ target: RidgeTarget, glide: Bool = false) throws -> Pixels {
        var list = RidgeDrawList()
        RidgeGeometryBuilder().build(frame, target: target, glide: glide, into: &list)
        return try render(list, width: target.pixelWidth, height: target.pixelHeight)
    }

    private func render(_ list: RidgeDrawList, width w: Int, height h: Int) throws -> Pixels {
        let desc = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: RidgeMetalRenderer.pixelFormat, width: w, height: h, mipmapped: false)
        desc.usage = [.renderTarget]
        desc.storageMode = .private
        let texture = try XCTUnwrap(renderer.device.makeTexture(descriptor: desc))
        let drawn = try XCTUnwrap(renderer.render(list, to: texture))
        drawn.waitUntilCompleted()
        XCTAssertEqual(drawn.status, .completed)
        XCTAssertNil(drawn.error)
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
        let bytes = Array(UnsafeBufferPointer(start: buffer.contents().assumingMemoryBound(to: UInt8.self), count: w * h * 4))
        return Pixels(width: w, height: h, bytes: bytes)
    }

    private let flat11 = [Float](repeating: 0, count: 11)
    private let ones11 = [Float](repeating: 1, count: 11)
    private let square = RidgeTarget(pixelWidth: 200, pixelHeight: 200, scale: 2)

    func testFlatRowsSitOnThePixelGridAWholeNumberOfPixelsApart() throws {
        let px = try render(makeRidgeFrame(rows: 5, params: .solid, edge: ones11) { _ in self.flat11 }, square)
        for k in 0..<5 {
            let top = 197 - 31 * k
            XCTAssertEqual(px.alpha(x: 100, y: top - 1), 0, "row \(k) above")
            XCTAssertEqual(px.alpha(x: 100, y: top), 255, "row \(k) first")
            XCTAssertEqual(px.alpha(x: 100, y: top + 1), 255, "row \(k) second")
            XCTAssertEqual(Int(px.alpha(x: 100, y: top + 2)), 128, accuracy: 2, "row \(k) half")
            XCTAssertEqual(px.alpha(x: 100, y: top + 3), 0, "row \(k) below")
        }
    }

    func testLineCoverageAddsUpToTheWidth() throws {
        for (target, width) in [(square, 2.5), (RidgeTarget(pixelWidth: 100, pixelHeight: 100, scale: 1), 1.25)] {
            let px = try render(makeRidgeFrame(rows: 1, params: .solid, edge: ones11) { _ in self.flat11 }, target)
            let column = (0..<target.pixelHeight).reduce(0) { $0 + Int(px.alpha(x: target.pixelWidth / 2, y: $1)) }
            XCTAssertEqual(Double(column), width * 255, accuracy: width * 255 * 0.02, "scale \(target.scale)")
        }
    }

    func testRowsAreButtCapped() throws {
        var p = RidgeParams.solid
        p.ridgeSpan = 0.5
        let px = try render(makeRidgeFrame(rows: 1, params: p, edge: ones11) { _ in self.flat11 }, square)
        XCTAssertEqual(px.alpha(x: 49, y: 197), 0)
        XCTAssertEqual(px.alpha(x: 50, y: 197), 255)
        XCTAssertEqual(px.alpha(x: 149, y: 197), 255)
        XCTAssertEqual(px.alpha(x: 150, y: 197), 0)
    }

    func testANearPeakErasesTheRowsBehindIt() throws {
        var p = RidgeParams.solid
        p.ridgeHeight = 0.2
        p.ridgeDepthScale = 1
        let bump: [Float] = [0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0]
        let px = try render(makeRidgeFrame(rows: 2, params: p, edge: ones11) { k in k == 0 ? bump : self.flat11 }, square)
        XCTAssertEqual(px.alpha(x: 10, y: 157), 255, "back rule left of the bump")
        XCTAssertEqual(px.alpha(x: 100, y: 157), 0, "back rule under the bump")
        XCTAssertEqual(px.alpha(x: 100, y: 158), 0, "back rule under the bump")
        XCTAssertEqual(px.alpha(x: 100, y: 77), 255, "front line at the top of the bump")
        XCTAssertEqual(px.alpha(x: 100, y: 120), 0, "nothing inside the bump")
    }

    func testRowAlphaFollowsTheFade() throws {
        var p = RidgeParams.standard
        p.ridgeLineWidth = 2
        let target = RidgeTarget(pixelWidth: 100, pixelHeight: 100, scale: 1)
        let px = try render(makeRidgeFrame(rows: 3, params: p, edge: [1, 1, 1, 1]) { _ in [0, 0, 0, 0] }, target)
        XCTAssertEqual(Int(px.alpha(x: 50, y: 98)), Int((0.85 * 255).rounded()), accuracy: 1)
        let mid = 0.85 * pow(0.5, 0.75) * 255
        XCTAssertEqual(Int(px.alpha(x: 50, y: 98 - 31)), Int(mid.rounded()), accuracy: 1)
    }

    func testJoinsDrawEachPixelOnce() throws {
        var p = RidgeParams.standard
        p.ridgeAlpha = 0.5
        p.ridgeBackAlpha = 0.5
        let zigzag: [Float] = (0..<11).map { $0 % 2 == 0 ? 0 : 1 }
        let px = try render(makeRidgeFrame(rows: 1, params: p, edge: ones11) { _ in zigzag }, square)
        let brightest = px.bytes.enumerated().filter { $0.offset % 4 == 3 }.map(\.element).max() ?? 0
        XCTAssertGreaterThanOrEqual(brightest, 120, "the line is drawn")
        XCTAssertLessThanOrEqual(brightest, 129, "no pixel blended twice")
    }

    func testLinesAboveTheFrameRenderWithoutError() throws {
        let tall = [Float](repeating: 3, count: 11)
        let px = try render(makeRidgeFrame(rows: 2, params: .solid, edge: ones11) { _ in tall }, square)
        XCTAssertEqual(px.alpha(x: 100, y: 0), 0, "the line is above the top edge")
    }

    func testRenderWithoutADrawableSkipsAndReleasesItsSlot() throws {
        let layer = CAMetalLayer()
        layer.device = renderer.device
        layer.pixelFormat = RidgeMetalRenderer.pixelFormat
        layer.drawableSize = .zero
        var list = RidgeDrawList()
        RidgeGeometryBuilder().build(makeRidgeFrame(rows: 3, edge: ones11) { _ in self.flat11 }, target: square, glide: false, into: &list)
        for _ in 0..<(RidgeMetalRenderer.framesInFlight + 2) {
            XCTAssertFalse(renderer.render(list, to: layer))
        }
        _ = try render(list, width: 200, height: 200)
    }
}
