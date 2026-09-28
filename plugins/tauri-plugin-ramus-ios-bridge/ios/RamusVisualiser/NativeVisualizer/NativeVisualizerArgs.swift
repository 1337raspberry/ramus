import Foundation

/// The spectrum's shape, fixed for the life of the app: bands per channel,
/// frames per second, and how long each band's level trails the audio it
/// measures, in seconds, the lowest band first (Rust `SpectrumLayout`).
public struct NativeSpectrumLayout: Decodable, Equatable {
    public let bandCount: Int
    public let fps: Int
    public let onsetDelays: [Double]

    public init(bandCount: Int, fps: Int, onsetDelays: [Double]) {
        self.bandCount = bandCount
        self.fps = fps
        self.onsetDelays = onsetDelays
    }
}

/// The backdrop's four corner colours as the frontend sends them, already
/// through its tone pass (`adjustedRgb`), as `[r, g, b]` in 0...255, and the
/// dim the web backdrop applies (`--ultrablur-opacity`).
public struct NativeBackdrop: Decodable, Equatable {
    public let topLeft: [UInt8]
    public let topRight: [UInt8]
    public let bottomLeft: [UInt8]
    public let bottomRight: [UInt8]
    public let opacity: Double

    /// The corners, or nil when any corner doesn't have exactly three
    /// channels.
    var corners: BackdropCorners? {
        func color(_ c: [UInt8]) -> BackdropColor? {
            c.count == 3 ? BackdropColor(red: c[0], green: c[1], blue: c[2]) : nil
        }
        guard let tl = color(topLeft), let tr = color(topRight),
              let bl = color(bottomLeft), let br = color(bottomRight)
        else { return nil }
        return BackdropCorners(topLeft: tl, topRight: tr, bottomLeft: bl, bottomRight: br)
    }

    /// No saturation or brightness change (the colours arrive toned), and
    /// the dim clamped to 0...1.
    var tone: BackdropTone {
        let dim = opacity.isFinite ? min(1, max(0, opacity)) : 1
        return BackdropTone(saturation: 1, brightness: 1, opacity: dim)
    }
}

/// What the `showNativeVisualizer` command carries: the ridge's tuning
/// values (the frontend's `VISUALIZER_PARAMS`), the backdrop, whether
/// playback is playing, and the spectrum layout Rust adds.
public struct NativeVisualizerShowArgs: Decodable {
    let params: RidgeParams
    public let backdrop: NativeBackdrop
    public let playing: Bool
    public let layout: NativeSpectrumLayout

    private enum CodingKeys: String, CodingKey { case params, backdrop, playing, layout }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        params = (try? c.decode(RidgeParams.self, forKey: .params)) ?? .standard
        backdrop = try c.decode(NativeBackdrop.self, forKey: .backdrop)
        playing = try c.decode(Bool.self, forKey: .playing)
        layout = try c.decode(NativeSpectrumLayout.self, forKey: .layout)
    }
}

/// What the `updateNativeVisualizer` command carries; each field is
/// optional and applies only when present.
public struct NativeVisualizerUpdateArgs: Decodable {
    public let backdrop: NativeBackdrop?
    public let playing: Bool?
    public let clearFrames: Bool?
}
