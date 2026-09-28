// Ported from ramusTV `RamusTV/Visualiser/RidgeFrameStats.swift`.

import Foundation

/// Frame-timing counters for a diagnostics readout, collected by
/// `RidgeView` on the main thread over the window since the last `take`.
struct RidgeFrameStats {
    /// Display-link callbacks.
    var ticks = 0
    /// Display frames that passed with no callback: the main thread was
    /// still busy when the frame's callback was due.
    var missed = 0
    /// Display period seen on the link, in ms.
    var periodMs = 0.0
    /// How late each callback started after its frame's timestamp, in ms.
    var lateMs: [Double] = []
    /// Step, draw and image hand-off of each paint, in ms.
    var paintMs: [Double] = []
    /// From the end of a paint to the end of the Core Animation commit that
    /// carries it, in ms.
    var commitMs: [Double] = []
    /// GPU time of each frame drawn with Metal, in ms.
    var gpuMs: [Double] = []
    /// Display frames between consecutive history-row cuts: index 0 counts
    /// gaps of one frame, index 3 gaps of four or more.
    var cutGaps = [0, 0, 0, 0]
    /// Seconds the window covers.
    var seconds = 0.0

    /// Paints per second over the window; 0 before any time has passed.
    var paintsPerSecond: Double {
        seconds > 0 ? Double(paintMs.count) / seconds : 0
    }

    /// One-line summary for a log.
    var summary: String {
        func dist(_ v: [Double]) -> String {
            guard !v.isEmpty else { return "-" }
            let s = v.sorted()
            let p50 = s[s.count / 2]
            let p95 = s[min(s.count - 1, Int(Double(s.count) * 0.95))]
            return String(format: "%.1f/%.1f/%.1f", p50, p95, s.last ?? 0)
        }
        let fps = paintsPerSecond
        return String(
            format: "period %.2f ms · ticks %d · missed %d · paints %.1f/s · paint %@ · commit %@ · gpu %@ · late %@ ms (p50/p95/max) · cut gaps 1:%d 2:%d 3:%d 4+:%d",
            periodMs, ticks, missed, fps, dist(paintMs), dist(commitMs), dist(gpuMs), dist(lateMs),
            cutGaps[0], cutGaps[1], cutGaps[2], cutGaps[3])
    }
}
