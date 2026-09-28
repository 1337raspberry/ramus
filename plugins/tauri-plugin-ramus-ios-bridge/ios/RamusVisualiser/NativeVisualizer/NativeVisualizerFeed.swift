import Foundation

/// The live inputs of one native visualiser: the frame ring, the audible
/// clock and whether playback is playing. The plugin writes it from Tauri's
/// command queue and the ridge reads it on the main thread; every member is
/// safe to use from either.
public final class NativeVisualizerFeed {
    let ring = SpectrumRing()
    let reader: SpectrumReader

    private let lock = NSLock()
    private var playing: Bool

    public init(layout: NativeSpectrumLayout, playing: Bool) {
        let period = 1 / Double(max(layout.fps, 1))
        reader = SpectrumReader(
            ring: ring,
            bandAhead: SpectrumReader.bandAhead(onsetDelays: layout.onsetDelays, framePeriodS: period),
            framePeriodS: period)
        self.playing = playing
        reader.isPlayingProvider = { [weak self] in self?.isPlaying ?? false }
    }

    /// Whether playback is playing; while false nothing is read and the
    /// line decays.
    public var isPlaying: Bool {
        get {
            lock.lock()
            defer { lock.unlock() }
            return playing
        }
        set {
            lock.lock()
            playing = newValue
            lock.unlock()
        }
    }

    /// Add a batch of frames, stamped with its arrival.
    public func push(_ batch: NativeSpectrumBatch) {
        push(batch, now: hostTimeMillis())
    }

    func push(_ batch: NativeSpectrumBatch, now: Double) {
        ring.push(epoch: batch.epoch, bandCount: batch.bandCount, channels: batch.channels, frames: batch.frames, now: now)
    }

    /// Record an audible-clock tick, stamped with its arrival.
    public func setAudible(_ tick: NativeAudibleTick) {
        setAudible(tick, now: hostTimeMillis())
    }

    func setAudible(_ tick: NativeAudibleTick, now: Double) {
        ring.setAudibleClock(epoch: tick.epoch, position: tick.position, now: now)
    }

    /// Drop every frame and the clock (playback stopped, queue cleared).
    public func clearFrames() {
        ring.clear()
    }
}
