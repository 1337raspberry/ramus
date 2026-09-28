// Ported from ramusTV `RamusTV/Backdrop/BackdropAnimator.swift`.

/// The backdrop's animated state: the corner colours on screen, and the
/// field's strength (1 is the full field, 0 is black), each easing toward a
/// target. This is pure state driven by `step(now:)`, in seconds.
///
/// Colour changes crossfade from whatever is on screen over
/// `BackdropField.transitionSeconds`, on `BackdropField.easeInOut`, as
/// ramus `ui/src/lib/ultraBlurField.ts`, `UltraBlurRenderer.setColors`
/// does:
/// - the first colours paint at once;
/// - the same colours again do nothing;
/// - a change mid-fade starts from the colours on screen, so it retargets
///   smoothly instead of jumping.
///
/// Strength changes (to or from black) run over `strengthSeconds` on the
/// same curve.
struct BackdropAnimator {
    /// Length of a fade to or from black, in seconds.
    static let strengthSeconds = 0.4

    private(set) var shown: BackdropFieldColors?
    private(set) var target: BackdropFieldColors?
    private var from: BackdropFieldColors?
    private var colorStart = 0.0

    private(set) var strength = 1.0
    private(set) var targetStrength = 1.0
    private var strengthFrom = 1.0
    private var strengthStart = 0.0

    /// True while a colour or strength fade is running.
    var isAnimating: Bool { from != nil || strength != targetStrength }

    /// Sets the colours to show. True when the picture changed.
    mutating func setColors(_ colors: BackdropFieldColors, now: Double, animated: Bool) -> Bool {
        guard colors != target else { return false }
        target = colors
        guard animated, let shown else {
            shown = colors
            from = nil
            return true
        }
        from = shown
        colorStart = now
        return true
    }

    /// Sets the strength to show. True when it changed.
    mutating func setStrength(_ value: Double, now: Double, animated: Bool) -> Bool {
        guard value != targetStrength else { return false }
        targetStrength = value
        if animated {
            strengthFrom = strength
            strengthStart = now
        } else {
            strength = value
        }
        return true
    }

    /// Advances the fades to `now`. True while one is still running.
    mutating func step(now: Double) -> Bool {
        if let from, let target {
            let k = BackdropField.easeInOut((now - colorStart) / BackdropField.transitionSeconds)
            shown = from.mixed(to: target, k)
            if k >= 1 {
                shown = target
                self.from = nil
            }
        }
        if strength != targetStrength {
            let k = BackdropField.easeInOut((now - strengthStart) / Self.strengthSeconds)
            strength = strengthFrom + (targetStrength - strengthFrom) * k
            if k >= 1 { strength = targetStrength }
        }
        return isAnimating
    }
}
