// Ported from ramusTV `RamusTV/Tap/SpectrumReader.swift` (without the
// playback-position fallback).

import Foundation

/// The ridge paint loop's view of the spectrum: on each display-link tick
/// it estimates where the audio will be when the paint reaches the screen
/// and reads the spectrum ring there.
///
/// The estimate comes from the audible clock (mpv's `audio-pts` with its
/// stream epoch, `SpectrumRing.audibleTarget`) extrapolated to `now` plus
/// the paint's lead. Each band is read `bandAhead` frames further on, as
/// its filter lags the audio (`band_onset_delays`), so a kick's body lands
/// with its click. Before the first audible tick, and once the last one is
/// stale, nothing is read and the line decays to silence; ticks arrive
/// about twenty times a second once the tap is installed. Nothing is read
/// while playback is not playing.
///
/// Port of the ring read in the paint loop of
/// `ui/src/components/FocusVisualizer.tsx` (`render`, the `isPlaying`
/// block), with the band read-ahead table from its `loadSpectrumLayout`
/// and the tolerances `FRAME_LAG_TOLERANCE_S` / `FRAME_LEAD_TOLERANCE_S`.
final class SpectrumReader: RidgeFrameSource {
    /// A frame this far behind the estimated playhead is still drawn.
    /// Covers tick jitter and a dropped burst without falling back to
    /// silence between bursts (`FRAME_LAG_TOLERANCE_S`).
    static let frameLagToleranceS = 0.25

    /// A frame this far ahead of the estimated playhead is still drawn:
    /// about three frames, absorbing the latency between a tick being taken
    /// and its arrival (`FRAME_LEAD_TOLERANCE_S`).
    static let frameLeadToleranceS = 0.05

    let ring: SpectrumRing
    /// Per band, how many frames further on it is read.
    let bandAhead: [UInt8]
    /// Seconds between frames.
    let framePeriodS: Double

    private let lock = NSLock()
    private var isPlayingSource: () -> Bool = { false }

    /// Whether audio is playing (not paused, stopped or idle). Called on
    /// every paint on the main thread, so it must be cheap.
    var isPlayingProvider: () -> Bool {
        get { locked { isPlayingSource } }
        set { locked { isPlayingSource = newValue } }
    }

    init(ring: SpectrumRing, bandAhead: [UInt8], framePeriodS: Double) {
        self.ring = ring
        self.bandAhead = bandAhead
        self.framePeriodS = framePeriodS
    }

    /// Each band's onset delay as a whole number of frames, rounded
    /// (`Math.round(d / period)` in `loadSpectrumLayout`).
    static func bandAhead(onsetDelays: [Double], framePeriodS: Double) -> [UInt8] {
        onsetDelays.map { delay in
            let steps = delay / framePeriodS
            guard steps.isFinite else { return 0 }
            return UInt8(min(max(steps.rounded(), 0), 255))
        }
    }

    var isPlaying: Bool {
        isPlayingProvider()
    }

    func bands(now: Double, leadSec: Double) -> [UInt8]? {
        guard isPlaying, let target = ring.audibleTarget(now: now, aheadSec: leadSec) else { return nil }
        return ring.readSpectrumBands(
            epoch: target.epoch,
            target: target.pos,
            lagSec: Self.frameLagToleranceS,
            leadSec: Self.frameLeadToleranceS,
            ahead: bandAhead,
            periodSec: framePeriodS)
    }

    private func locked<T>(_ body: () -> T) -> T {
        lock.lock()
        defer { lock.unlock() }
        return body()
    }
}
