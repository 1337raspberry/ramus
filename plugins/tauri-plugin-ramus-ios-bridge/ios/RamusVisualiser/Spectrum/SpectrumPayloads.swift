import Foundation

/// One spectrum frame: the track-timeline position of its sample, in
/// seconds, and one quantised level (0...255) per band per channel, the
/// left channel's bands followed by the right channel's. Matches
/// `ramus_core::playback::spectrum_tap::SpectrumFrame` on the wire.
public struct SpectrumFrame: Decodable, Equatable {
    public var pos: Double
    public var bands: [UInt8]

    public init(pos: Double, bands: [UInt8]) {
        self.pos = pos
        self.bands = bands
    }
}

/// A batch of frames of one stream, as the `pushSpectrumFrames` plugin
/// command carries it (Rust `SpectrumFramesPayload`).
public struct NativeSpectrumBatch: Decodable {
    public let epoch: UInt64
    public let bandCount: Int
    public let channels: Int
    public let frames: [SpectrumFrame]

    public init(epoch: UInt64, bandCount: Int, channels: Int, frames: [SpectrumFrame]) {
        self.epoch = epoch
        self.bandCount = bandCount
        self.channels = channels
        self.frames = frames
    }
}

/// An audible-clock tick, as the `pushAudible` plugin command carries it
/// (Rust `PlaybackAudiblePayload`): the position being heard and its
/// stream epoch.
public struct NativeAudibleTick: Decodable {
    public let epoch: UInt64
    public let position: Double

    public init(epoch: UInt64, position: Double) {
        self.epoch = epoch
        self.position = position
    }
}
