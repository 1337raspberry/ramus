// Ported from ramusTV `RamusTV/Visualiser/RidgeFrameSource.swift`.

import Foundation

/// What the ridge paint loop reads on every display-link tick.
///
/// The tap side (`Tap/`) implements this over the spectrum ring and the
/// audible clock; the visualiser side (`Visualiser/`) only consumes it, so
/// the two never depend on each other's types.
protocol RidgeFrameSource: AnyObject {
    /// True while audio is playing (not paused, stopped or idle).
    var isPlaying: Bool { get }

    /// The bands to draw for a paint that will be on screen `leadSec` after
    /// `now`, or nil when the ring holds no frame near that moment.
    ///
    /// `now` is milliseconds on the `CACurrentMediaTime()` clock. The result
    /// holds one quantised level (0...255) per band per channel: the left
    /// channel's bands followed by the right channel's.
    func bands(now: Double, leadSec: Double) -> [UInt8]?
}
