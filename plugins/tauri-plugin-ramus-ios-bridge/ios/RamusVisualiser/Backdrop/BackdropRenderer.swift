// Ported from ramusTV `RamusTV/Backdrop/BackdropRenderer.swift`.

import Metal
import QuartzCore

/// Draws the backdrop field with Metal (`backdropShaderSource`) in two passes.
/// The field is smooth, so it is computed in float on a `gridSize` ×
/// `gridSize` grid of samples spanning the target, stored as half floats.
/// The second pass samples that grid with bilinear filtering at every target
/// pixel and adds a static dither once before the 8-bit store. The dither
/// is read from a texture made once per target size. Per pixel that costs
/// about as much as a flat fill: computing the field and hashing the dither
/// at every pixel took several times the GPU time at 4K. Interpolating
/// between samples stays within 0.2 of an 8-bit step for any corner colours.
///
/// The target must map one pixel to one screen pixel, because any
/// resampling on the way to the screen averages the dither away.
///
/// One frame is on the GPU at a time. A draw asked for while the previous
/// frame is still running is skipped, so the caller never blocks.
final class BackdropRenderer {
    static let pixelFormat = MTLPixelFormat.bgra8Unorm
    /// Samples along each axis of the field's grid, whatever the target's
    /// size.
    static let gridSize = 257

    let device: MTLDevice
    private let queue: MTLCommandQueue
    private let fieldPipeline: MTLRenderPipelineState
    private let ditherPipeline: MTLRenderPipelineState
    private let presentPipeline: MTLRenderPipelineState
    private let grid: MTLTexture
    private var dither: MTLTexture?
    private var inFlight: MTLCommandBuffer?

    /// Nil when there is no Metal device or the shaders fail to load.
    init?(device: MTLDevice? = MTLCreateSystemDefaultDevice()) {
        guard let device, let queue = device.makeCommandQueue() else { return nil }
        let gridDescriptor = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .rgba16Float, width: Self.gridSize, height: Self.gridSize, mipmapped: false)
        gridDescriptor.usage = [.renderTarget, .shaderRead]
        gridDescriptor.storageMode = .private
        guard let grid = device.makeTexture(descriptor: gridDescriptor) else { return nil }
        do {
            let library = try VisualiserShaders.library(.backdrop, on: device)
            fieldPipeline = try Self.pipeline(device, library, fragment: "backdropFieldFragment", format: .rgba16Float)
            ditherPipeline = try Self.pipeline(device, library, fragment: "backdropDitherFragment", format: .r8Snorm)
            presentPipeline = try Self.pipeline(device, library, fragment: "backdropFragment", format: Self.pixelFormat)
        } catch {
            Log.visualiser.error("backdrop shader failed to load: \(String(describing: error), privacy: .public)")
            return nil
        }
        self.device = device
        self.queue = queue
        self.grid = grid
    }

    private static func pipeline(
        _ device: MTLDevice, _ library: MTLLibrary, fragment: String, format: MTLPixelFormat
    ) throws -> MTLRenderPipelineState {
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.vertexFunction = library.makeFunction(name: "backdropVertex")
        descriptor.fragmentFunction = library.makeFunction(name: fragment)
        descriptor.colorAttachments[0].pixelFormat = format
        return try device.makeRenderPipelineState(descriptor: descriptor)
    }

    /// Draws into the layer's next drawable and presents it. False when the
    /// frame was skipped: the previous frame is still on the GPU, or no
    /// drawable was available.
    func render(_ uniforms: BackdropUniforms, to layer: CAMetalLayer) -> Bool {
        guard !isBusy, let drawable = layer.nextDrawable() else { return false }
        return encode(uniforms, into: drawable.texture, present: drawable) != nil
    }

    /// Draws into `texture` and commits; the returned command buffer is
    /// already committed. Nil when the frame was skipped.
    @discardableResult
    func render(_ uniforms: BackdropUniforms, to texture: MTLTexture) -> MTLCommandBuffer? {
        guard !isBusy else { return nil }
        return encode(uniforms, into: texture, present: nil)
    }

    /// True while the previous frame is still on the GPU.
    private var isBusy: Bool {
        guard let status = inFlight?.status else { return false }
        return status != .completed && status != .error
    }

    private func encode(_ uniforms: BackdropUniforms, into texture: MTLTexture, present drawable: MTLDrawable?) -> MTLCommandBuffer? {
        guard let commandBuffer = queue.makeCommandBuffer() else { return nil }
        var dither = self.dither
        if dither?.width != texture.width || dither?.height != texture.height {
            dither = makeDither(width: texture.width, height: texture.height, in: commandBuffer)
            self.dither = dither
        }
        guard let dither else { return nil }

        var u = uniforms
        u.viewport = SIMD2(Float(Self.gridSize), Float(Self.gridSize))
        guard let field = commandBuffer.makeRenderCommandEncoder(descriptor: Self.pass(grid)) else { return nil }
        field.setRenderPipelineState(fieldPipeline)
        field.setFragmentBytes(&u, length: MemoryLayout<BackdropUniforms>.stride, index: 0)
        field.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
        field.endEncoding()

        u.viewport = SIMD2(Float(texture.width), Float(texture.height))
        guard let present = commandBuffer.makeRenderCommandEncoder(descriptor: Self.pass(texture)) else { return nil }
        present.setRenderPipelineState(presentPipeline)
        present.setFragmentBytes(&u, length: MemoryLayout<BackdropUniforms>.stride, index: 0)
        present.setFragmentTexture(grid, index: 0)
        present.setFragmentTexture(dither, index: 1)
        present.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
        present.endEncoding()

        if let drawable { commandBuffer.present(drawable) }
        commandBuffer.commit()
        inFlight = commandBuffer
        return commandBuffer
    }

    /// Makes the dither texture for a target size and encodes the pass that
    /// fills it, ahead of the frame's own passes.
    private func makeDither(width: Int, height: Int, in commandBuffer: MTLCommandBuffer) -> MTLTexture? {
        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .r8Snorm, width: width, height: height, mipmapped: false)
        descriptor.usage = [.renderTarget, .shaderRead]
        descriptor.storageMode = .private
        guard let texture = device.makeTexture(descriptor: descriptor),
              let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: Self.pass(texture))
        else { return nil }
        encoder.setRenderPipelineState(ditherPipeline)
        encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 3)
        encoder.endEncoding()
        return texture
    }

    /// A pass that overwrites every pixel of `texture`.
    private static func pass(_ texture: MTLTexture) -> MTLRenderPassDescriptor {
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = texture
        pass.colorAttachments[0].loadAction = .dontCare
        pass.colorAttachments[0].storeAction = .store
        return pass
    }
}
