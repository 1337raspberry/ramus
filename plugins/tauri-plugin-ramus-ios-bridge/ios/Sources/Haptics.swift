import CoreHaptics
import UIKit
import os

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "haptics")

/// Every strength the app's haptics use, in one place.
private enum Strength {
    static let pop: (UIImpactFeedbackGenerator.FeedbackStyle, CGFloat) = (.light, 0.6)
    static let thud: (UIImpactFeedbackGenerator.FeedbackStyle, CGFloat) = (.soft, 0.8)
    static let bump: (UIImpactFeedbackGenerator.FeedbackStyle, CGFloat) = (.rigid, 0.7)
    static let holdFire: (UIImpactFeedbackGenerator.FeedbackStyle, CGFloat) = (.medium, 0.8)

    static let grain: (UIImpactFeedbackGenerator.FeedbackStyle, CGFloat) = (.rigid, 0.4)

    /// The hold rumble climbs from `rumbleStart` to `rumblePeak`. It starts
    /// high enough to be felt at once: it warns of a hold in time to let go.
    static let rumbleStart: Float = 0.2
    static let rumblePeak: Float = 0.5
    static let rumbleSharpness: Float = 0.2
    /// Rumbles shorter than this are skipped: too brief to read as a build.
    static let rumbleMinMs = 100.0
}

/// Plays the named haptic effects the page asks for. Main queue only.
///
/// Taps use `UIImpactFeedbackGenerator`; the rumble that builds through a
/// pending long press is a Core Haptics pattern, the one effect that needs an
/// intensity curve.
final class Haptics {
    private var generators: [UIImpactFeedbackGenerator.FeedbackStyle: UIImpactFeedbackGenerator] = [:]
    private let rumble = HoldRumble()

    /// `value` is the rumble's length in milliseconds for `holdStart`.
    /// Unknown names are ignored.
    func play(effect: String, value: Double?) {
        switch effect {
        case "pop": impact(Strength.pop)
        case "thud": impact(Strength.thud)
        case "bump": impact(Strength.bump)
        case "grain": impact(Strength.grain)
        case "holdStart":
            rumble.start(ms: value ?? 0)
        case "holdCancel":
            rumble.stop()
        case "holdFire":
            rumble.stop()
            impact(Strength.holdFire)
        default:
            break
        }
    }

    private func impact(_ strength: (UIImpactFeedbackGenerator.FeedbackStyle, CGFloat)) {
        let (style, intensity) = strength
        let generator: UIImpactFeedbackGenerator
        if let existing = generators[style] {
            generator = existing
        } else {
            generator = UIImpactFeedbackGenerator(style: style)
            generators[style] = generator
        }
        generator.impactOccurred(intensity: intensity)
        // Keeps the Taptic Engine spun up for a follow-on tap (a scrub sends
        // many), which would otherwise land late.
        generator.prepare()
    }
}

/// The rumble under a pending long press: a continuous vibration whose
/// intensity climbs until the hold fires.
///
/// All engine work happens on a private queue: starting a stopped engine can
/// block for a while, which the main thread must not pay for mid-gesture.
private final class HoldRumble: @unchecked Sendable {
    private let queue = DispatchQueue(label: "com.raspsoft.ramus.haptics.rumble")
    private let supported = CHHapticEngine.capabilitiesForHardware().supportsHaptics
    // Queue-confined state.
    private var engine: CHHapticEngine?
    private var player: CHHapticPatternPlayer?
    private var reportedFailure = false

    func start(ms: Double) {
        guard supported, ms >= Strength.rumbleMinMs else { return }
        queue.async {
            self.stopPlayer()
            self.play(duration: ms / 1000)
        }
    }

    func stop() {
        guard supported else { return }
        queue.async { self.stopPlayer() }
    }

    private func stopPlayer() {
        try? player?.stop(atTime: CHHapticTimeImmediate)
        player = nil
    }

    private func play(duration: TimeInterval) {
        do {
            let engine = try self.engine ?? makeEngine()
            self.engine = engine
            // A no-op while the engine runs; restarts it after the system
            // stopped it (backgrounding, idle shutdown, a server reset).
            try engine.start()
            let event = CHHapticEvent(
                eventType: .hapticContinuous,
                parameters: [
                    CHHapticEventParameter(parameterID: .hapticIntensity, value: 1),
                    CHHapticEventParameter(
                        parameterID: .hapticSharpness, value: Strength.rumbleSharpness),
                ],
                relativeTime: 0,
                duration: duration)
            let ramp = CHHapticParameterCurve(
                parameterID: .hapticIntensityControl,
                controlPoints: [
                    .init(relativeTime: 0, value: Strength.rumbleStart),
                    .init(relativeTime: duration, value: Strength.rumblePeak),
                ],
                relativeTime: 0)
            let pattern = try CHHapticPattern(events: [event], parameterCurves: [ramp])
            let player = try engine.makePlayer(with: pattern)
            try player.start(atTime: CHHapticTimeImmediate)
            self.player = player
        } catch {
            // Build a fresh engine next time rather than keep one that may
            // never start again.
            engine = nil
            player = nil
            if !reportedFailure {
                reportedFailure = true
                log.error("hold rumble unavailable: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    private func makeEngine() throws -> CHHapticEngine {
        let engine = try CHHapticEngine()
        // Haptics only: the engine must never join or reconfigure the audio
        // session that playback owns.
        engine.playsHapticsOnly = true
        engine.isAutoShutdownEnabled = true
        return engine
    }
}
