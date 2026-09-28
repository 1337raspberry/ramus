import Foundation
import Metal

/// Why the native visualiser can't draw.
public enum VisualiserError: Error {
    /// The system has no Metal device.
    case noMetalDevice
}

/// The visualiser's Metal libraries, compiled from source at runtime.
///
/// The plugin's Swift sources are compiled into a static library by the
/// Tauri build rather than by Xcode, so there is no bundle to carry a
/// precompiled `default.metallib`; the shader text ships inside the binary
/// (`ridgeShaderSource`, `backdropShaderSource`) instead. Each library is
/// compiled once per device and kept for the life of the process, which
/// takes a few tens of milliseconds the first time.
public enum VisualiserShaders {
    enum Kind {
        case ridge
        case backdrop

        var source: String {
            switch self {
            case .ridge: return ridgeShaderSource
            case .backdrop: return backdropShaderSource
            }
        }
    }

    private static let lock = NSLock()
    private static var cache: [Kind: (device: ObjectIdentifier, library: MTLLibrary)] = [:]

    /// The compiled library for `kind` on `device`, compiling it on first use.
    static func library(_ kind: Kind, on device: MTLDevice) throws -> MTLLibrary {
        lock.lock()
        defer { lock.unlock() }
        let id = ObjectIdentifier(device as AnyObject)
        if let hit = cache[kind], hit.device == id { return hit.library }
        let library = try device.makeLibrary(source: kind.source, options: nil)
        cache[kind] = (id, library)
        return library
    }

    /// Compiles both libraries for the system's default device ahead of
    /// use. Call off the main thread; throws the first failure.
    public static func prepare() throws {
        guard let device = MTLCreateSystemDefaultDevice() else { throw VisualiserError.noMetalDevice }
        _ = try library(.ridge, on: device)
        _ = try library(.backdrop, on: device)
    }
}
