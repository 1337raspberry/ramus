// Ported from ramusTV `RamusTV/Tap/SpectrumRing.swift`.

import Foundation
import QuartzCore

/// Milliseconds on the host clock, `CACurrentMediaTime()`: the time base
/// of the spectrum ring, the audible clock and `RidgeFrameSource`.
func hostTimeMillis() -> Double {
    CACurrentMediaTime() * 1000
}

/// One frame held by `SpectrumRing`.
///
/// Port of `ui/src/lib/spectrumRing.ts::SpectrumRingFrame`.
struct SpectrumRingFrame: Equatable {
    /// Stream epoch the frame belongs to.
    var epoch: UInt64
    /// Track-timeline position of the frame's sample, in seconds.
    var pos: Double
    /// One quantised level (0...255) per band per channel: the left
    /// channel's bands followed by the right channel's.
    var bands: [UInt8]
}

/// Where the audio will be at a given moment: the epoch to look in and the
/// position within it.
struct AudibleTarget: Equatable {
    let epoch: UInt64
    let pos: Double
}

/// Position-keyed ring of live spectrum frames, plus the audible clock the
/// paint loop reads them against.
///
/// Frames arrive in bursts, up to mpv's `audio-buffer` ahead of what is
/// being heard. The ring holds about two seconds at 60 fps so the lookup
/// always has the frame for "now" with margin on both sides, even across a
/// gapless join where the incoming stream's frames share the ring with the
/// outgoing stream's still-audible tail; a seek's stale frames simply age
/// out.
///
/// Every frame carries the stream epoch it was cut from, and the lookup is
/// clocked by the audible position (mpv's `audio-pts`) rather than the
/// seek-bar position: across a gapless join mpv moves to the next track
/// about an audio buffer before the join is heard, and the audible position
/// runs negative on the new track's timeline through that window. Mapping a
/// negative audible position back onto the end of the previous epoch keeps
/// the picture on the outgoing track until the join is actually heard.
///
/// Written from the plugin's command queue and read from the display link on
/// the main thread, so every access takes the lock. Times are milliseconds
/// on `CACurrentMediaTime()` (`hostTimeMillis()`); each method that reads
/// the clock takes `now` explicitly, defaulting to the real one.
///
/// Port of `ui/src/lib/spectrumRing.ts`.
final class SpectrumRing {
    /// Frames held (`RING_CAPACITY`).
    static let capacity = 128

    /// A clock tick older than this is no clock at all. Ticks land about
    /// twenty times a second while audio flows, so a gap this long means
    /// the stream behind the clock has stopped: a skip has replaced it, or
    /// playback paused. Past it the reader draws nothing rather than
    /// running the dead stream's timeline on, which would draw the frames
    /// the tap cut ahead of the last audible moment
    /// (`AUDIBLE_CLOCK_STALE_MS`).
    static let audibleClockStaleMs: Double = 250

    /// A snapshot for a diagnostics readout.
    struct Stats: Equatable {
        /// Frames pushed since the ring was created; never reset, so two
        /// snapshots a second apart give the frame rate.
        var framesPushed: Int
        /// How far the newest frame of the audible epoch sits ahead of the
        /// audible position, in seconds; nil without a live clock or
        /// frames in that epoch.
        var leadSec: Double?
    }

    private struct AudibleClock {
        let epoch: UInt64
        let position: Double
        /// Host milliseconds when the tick arrived.
        let at: Double
    }

    private let lock = NSLock()
    /// Slots `0..<count` are the filled ones: `head` starts at 0 and a
    /// clear resets it to 0, so the filled region is always a prefix.
    private var ring = [SpectrumRingFrame](repeating: SpectrumRingFrame(epoch: 0, pos: 0, bands: []), count: SpectrumRing.capacity)
    private var head = 0
    private var count = 0
    private var lastPush: Double = 0
    private var pushed = 0
    /// Position of the latest frame seen per epoch, for the current epoch
    /// and the one before it: the end of the previous stream is where a
    /// negative audible position on the current one maps to.
    private var epochEnd: [UInt64: Double] = [:]
    private var clock: AudibleClock?

    init() {}

    /// Host milliseconds of the most recent push; 0 if none yet
    /// (`spectrumLastPushAt`).
    var lastPushAt: Double {
        locked { lastPush }
    }

    /// Append a burst of frames (oldest first), all of stream `epoch`
    /// (`pushSpectrumFrames`). A frame whose width is not
    /// `bandCount * channels` is dropped rather than drawn as noise.
    func push(epoch: UInt64, bandCount: Int, channels: Int, frames: [SpectrumFrame], now: Double = hostTimeMillis()) {
        let width = bandCount * channels
        locked {
            for frame in frames where frame.bands.count == width {
                ring[head] = SpectrumRingFrame(epoch: epoch, pos: frame.pos, bands: frame.bands)
                head = (head + 1) % Self.capacity
                if count < Self.capacity { count += 1 }
                pushed += 1
                if let end = epochEnd[epoch], !(frame.pos > end) {
                    continue
                }
                epochEnd[epoch] = frame.pos
            }
            if epoch > 0 {
                for old in epochEnd.keys where old < epoch - 1 {
                    epochEnd[old] = nil
                }
            }
            lastPush = now
        }
    }

    /// Drop every frame and the audible clock (playback stopped, queue
    /// replaced) (`clearSpectrumRing`).
    func clear() {
        locked {
            head = 0
            count = 0
            epochEnd.removeAll()
            clock = nil
        }
    }

    /// Record an audible tick: the position being heard and its epoch,
    /// arriving at `now` (`setAudibleClock`).
    func setAudibleClock(epoch: UInt64, position: Double, now: Double = hostTimeMillis()) {
        locked {
            clock = AudibleClock(epoch: epoch, position: position, at: now)
        }
    }

    /// Where the audio will be `aheadSec` after `now`, extrapolated from
    /// the last audible tick. A negative position on the current epoch is
    /// still the previous stream's tail and is mapped onto the end of that
    /// stream. Nil until the first tick arrives, and again once the last
    /// tick is stale at `now` (`audibleClockStaleMs`) (`audibleTarget`).
    func audibleTarget(now: Double = hostTimeMillis(), aheadSec: Double = 0) -> AudibleTarget? {
        locked { target(now: now, aheadSec: aheadSec) }
    }

    /// The frame of `epoch` (any epoch when nil) closest to `target`
    /// seconds within `[target - lagSec, target + leadSec]`, or nil when
    /// nothing is that close (`pickSpectrumFrame`).
    func pickSpectrumFrame(epoch: UInt64?, target: Double, lagSec: Double, leadSec: Double) -> SpectrumRingFrame? {
        locked {
            pickIndex(epoch: epoch, target: target, lagSec: lagSec, leadSec: leadSec).map { ring[$0] }
        }
    }

    /// The levels to draw for `target`: the frame `pickSpectrumFrame`
    /// finds, with each band read instead from the frame `ahead[k]` frames
    /// after it in the same stream (`periodSec` apart). A narrow bass
    /// band's level swells for tens of milliseconds after the note it
    /// measures starts, so reading it that much further on lands its onsets
    /// with the treble's. `ahead` holds one step count per band and applies
    /// on every channel; a later frame the ring doesn't hold (the stream
    /// ended, or a seek) falls back to the latest one before it that it
    /// does. Nil when there is no frame near `target`
    /// (`readSpectrumBands`).
    func readSpectrumBands(
        epoch: UInt64?,
        target: Double,
        lagSec: Double,
        leadSec: Double,
        ahead: [UInt8]?,
        periodSec: Double
    ) -> [UInt8]? {
        locked {
            guard let baseIndex = pickIndex(epoch: epoch, target: target, lagSec: lagSec, leadSec: leadSec) else {
                return nil
            }
            let base = ring[baseIndex]
            let width = base.bands.count
            guard let ahead, !ahead.isEmpty, width % ahead.count == 0, periodSec > 0 else {
                return base.bands
            }
            let n = ahead.count
            let maxStep = Int(ahead.max() ?? 0)
            if maxStep == 0 { return base.bands }

            // Ring slot of the frame at each step past the base frame.
            var steps = [Int](repeating: baseIndex, count: maxStep + 1)
            for m in 1...maxStep {
                let want = base.pos + Double(m) * periodSec
                var found = steps[m - 1]
                for i in 0..<count
                where ring[i].epoch == base.epoch
                    && ring[i].bands.count == width
                    && abs(ring[i].pos - want) < periodSec / 2 {
                    found = i
                    break
                }
                steps[m] = found
            }

            var composite = [UInt8](repeating: 0, count: width)
            var c = 0
            while c < width {
                for k in 0..<n {
                    composite[c + k] = ring[steps[Int(ahead[k])]].bands[c + k]
                }
                c += n
            }
            return composite
        }
    }

    /// Frame count and the ring's lead over the audible clock at `now`.
    func stats(now: Double = hostTimeMillis()) -> Stats {
        locked {
            var lead: Double?
            if let target = target(now: now, aheadSec: 0), let end = epochEnd[target.epoch] {
                lead = end - target.pos
            }
            return Stats(framesPushed: pushed, leadSec: lead)
        }
    }

    // MARK: - Unlocked internals

    private func target(now: Double, aheadSec: Double) -> AudibleTarget? {
        guard let clock, !(now - clock.at > Self.audibleClockStaleMs) else { return nil }
        var epoch = clock.epoch
        var pos = clock.position + (now - clock.at) / 1000 + aheadSec
        if pos < 0, epoch > 0, let end = epochEnd[epoch - 1] {
            epoch -= 1
            pos += end
        }
        return AudibleTarget(epoch: epoch, pos: pos)
    }

    private func pickIndex(epoch: UInt64?, target: Double, lagSec: Double, leadSec: Double) -> Int? {
        var best: Int?
        var bestDist = Double.infinity
        for i in 0..<count {
            if let epoch, ring[i].epoch != epoch { continue }
            let d = ring[i].pos - target
            if d < -lagSec || d > leadSec { continue }
            let dist = abs(d)
            if dist < bestDist {
                bestDist = dist
                best = i
            }
        }
        return best
    }

    private func locked<T>(_ body: () throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body()
    }
}
