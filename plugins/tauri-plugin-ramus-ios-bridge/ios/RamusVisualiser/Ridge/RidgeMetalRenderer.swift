// Ported from ramusTV `RamusTV/Visualiser/RidgeMetalRenderer.swift`.

import Metal
import QuartzCore

/// Uniforms shared by the ridge shaders; the layout matches
/// `RidgeUniforms` in `ridgeShaderSource`.
struct RidgeUniforms {
    var color: SIMD4<Float>
    var viewport: SIMD2<Float>
    var lineWidth: Float
    var padding: Float = 0
}

/// Draws a `RidgeDrawList` with Metal: each row's erase (destination-out,
/// so the layer turns transparent where a nearer row rises) and then its
/// stroke (premultiplied source-over), back to front, over a target cleared
/// to transparent.
///
/// Up to `framesInFlight` frames can be on the GPU at once, each with its
/// own vertex buffers. A frame that would wait for one to free up is
/// skipped instead, so the caller's thread never blocks on the GPU.
final class RidgeMetalRenderer {
    static let pixelFormat = MTLPixelFormat.bgra8Unorm
    static let framesInFlight = 3

    let device: MTLDevice
    /// Called on the main queue with each frame's GPU time, in ms.
    var onGPUTime: ((Double) -> Void)?

    private let queue: MTLCommandQueue
    private let erasePipeline: MTLRenderPipelineState
    private let strokePipeline: MTLRenderPipelineState
    private let inFlight = DispatchSemaphore(value: RidgeMetalRenderer.framesInFlight)
    private var spanBuffers = [MTLBuffer?](repeating: nil, count: RidgeMetalRenderer.framesInFlight)
    private var segmentBuffers = [MTLBuffer?](repeating: nil, count: RidgeMetalRenderer.framesInFlight)
    private var slot = 0

    /// Nil when there is no Metal device or the shaders fail to load.
    init?(device: MTLDevice? = MTLCreateSystemDefaultDevice()) {
        guard let device, let queue = device.makeCommandQueue() else { return nil }
        do {
            let library = try VisualiserShaders.library(.ridge, on: device)
            erasePipeline = try Self.pipeline(
                device, library, vertex: "ridgeEraseVertex", fragment: "ridgeEraseFragment", erase: true)
            strokePipeline = try Self.pipeline(
                device, library, vertex: "ridgeStrokeVertex", fragment: "ridgeStrokeFragment", erase: false)
        } catch {
            Log.visualiser.error("ridge shaders failed to load: \(String(describing: error), privacy: .public)")
            return nil
        }
        self.device = device
        self.queue = queue
    }

    private static func pipeline(
        _ device: MTLDevice, _ library: MTLLibrary, vertex: String, fragment: String, erase: Bool
    ) throws -> MTLRenderPipelineState {
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.vertexFunction = library.makeFunction(name: vertex)
        descriptor.fragmentFunction = library.makeFunction(name: fragment)
        let color = descriptor.colorAttachments[0]!
        color.pixelFormat = pixelFormat
        color.isBlendingEnabled = true
        color.rgbBlendOperation = .add
        color.alphaBlendOperation = .add
        color.sourceRGBBlendFactor = erase ? .zero : .one
        color.sourceAlphaBlendFactor = erase ? .zero : .one
        color.destinationRGBBlendFactor = .oneMinusSourceAlpha
        color.destinationAlphaBlendFactor = .oneMinusSourceAlpha
        return try device.makeRenderPipelineState(descriptor: descriptor)
    }

    /// Draws `list` into the layer's next drawable and presents it. False
    /// when the frame was skipped: no drawable, or `framesInFlight` frames
    /// still on the GPU.
    func render(_ list: RidgeDrawList, to layer: CAMetalLayer) -> Bool {
        guard inFlight.wait(timeout: .now()) == .success else { return false }
        guard let drawable = layer.nextDrawable() else {
            inFlight.signal()
            return false
        }
        return commit(list, to: drawable.texture, present: drawable) != nil
    }

    /// Draws `list` into `texture` and commits; the returned command buffer
    /// is already committed. Nil when the frame was skipped.
    @discardableResult
    func render(_ list: RidgeDrawList, to texture: MTLTexture) -> MTLCommandBuffer? {
        guard inFlight.wait(timeout: .now()) == .success else { return nil }
        return commit(list, to: texture, present: nil)
    }

    /// Encodes and commits one frame; the caller holds an in-flight slot,
    /// which the command buffer's completion releases.
    private func commit(_ list: RidgeDrawList, to texture: MTLTexture, present drawable: MTLDrawable?) -> MTLCommandBuffer? {
        guard let commandBuffer = queue.makeCommandBuffer() else {
            inFlight.signal()
            return nil
        }
        let semaphore = inFlight
        let report = onGPUTime
        commandBuffer.addCompletedHandler { buffer in
            semaphore.signal()
            guard let report, buffer.gpuEndTime > buffer.gpuStartTime else { return }
            let ms = (buffer.gpuEndTime - buffer.gpuStartTime) * 1000
            DispatchQueue.main.async { report(ms) }
        }
        slot = (slot + 1) % Self.framesInFlight
        let spans = upload(list.spans, into: &spanBuffers[slot])
        let segments = upload(list.segments, into: &segmentBuffers[slot])

        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = texture
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].clearColor = MTLClearColor(red: 0, green: 0, blue: 0, alpha: 0)
        pass.colorAttachments[0].storeAction = .store
        if let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: pass) {
            var uniforms = RidgeUniforms(
                color: SIMD4(Float(list.color.red), Float(list.color.green), Float(list.color.blue), 1),
                viewport: SIMD2(Float(texture.width), Float(texture.height)),
                lineWidth: list.lineWidth)
            encoder.setVertexBytes(&uniforms, length: MemoryLayout<RidgeUniforms>.stride, index: 1)
            encoder.setFragmentBytes(&uniforms, length: MemoryLayout<RidgeUniforms>.stride, index: 1)
            let spanStride = MemoryLayout<RidgeEraseSpan>.stride
            let segmentStride = MemoryLayout<RidgeStrokeSegment>.stride
            for row in list.rows {
                if let spans, !row.spans.isEmpty {
                    encoder.setRenderPipelineState(erasePipeline)
                    encoder.setVertexBuffer(spans, offset: row.spans.lowerBound * spanStride, index: 0)
                    encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 6, instanceCount: row.spans.count)
                }
                if let segments, !row.segments.isEmpty {
                    encoder.setRenderPipelineState(strokePipeline)
                    encoder.setVertexBuffer(segments, offset: row.segments.lowerBound * segmentStride, index: 0)
                    encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 6, instanceCount: row.segments.count)
                }
            }
            encoder.endEncoding()
        }
        if let drawable { commandBuffer.present(drawable) }
        commandBuffer.commit()
        return commandBuffer
    }

    /// Copies `items` into `buffer`, growing it to fit; nil for no items.
    private func upload<T>(_ items: [T], into buffer: inout MTLBuffer?) -> MTLBuffer? {
        guard !items.isEmpty else { return nil }
        let length = items.count * MemoryLayout<T>.stride
        if (buffer?.length ?? 0) < length {
            buffer = device.makeBuffer(length: length * 3 / 2, options: .storageModeShared)
        }
        guard let buffer else { return nil }
        items.withUnsafeBytes { buffer.contents().copyMemory(from: $0.baseAddress!, byteCount: length) }
        return buffer
    }
}
