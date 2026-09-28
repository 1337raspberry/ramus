// Ported from ramusTV `RamusTV/Backdrop/BackdropField.swift`.

import Foundation
import simd

/// The backdrop's tone: how far the corner colours are saturated and
/// brightened, and how opaque the field is over the page colour. These are
/// ramus's two display profiles
/// (`ui/src/components/UltraBlurBackground.tsx` `SATURATION` and
/// `BRIGHTNESS`, `ui/src/styles.css` `--ultrablur-opacity`), which it picks
/// with the `dynamic-range` media query.
struct BackdropTone: Equatable {
    var saturation: Double
    var brightness: Double
    var opacity: Double

    static let sdr = BackdropTone(saturation: 1.3, brightness: 1.0, opacity: 0.95)
    static let hdr = BackdropTone(saturation: 1.05, brightness: 0.9, opacity: 0.8)

    /// Saturation as a per-channel blend away from the RGB mean, then
    /// brightness as a scalar multiply, rounded and clamped to 8 bits
    /// (ramus `ui/src/components/UltraBlurBackground.tsx`, `adjustedRgb`).
    func adjusted(_ c: BackdropColor) -> BackdropColor {
        let r = Double(c.red), g = Double(c.green), b = Double(c.blue)
        let grey = (r + g + b) / 3
        func channel(_ v: Double) -> UInt8 {
            UInt8(max(0, min(255, ((grey + (v - grey) * saturation) * brightness).rounded())))
        }
        return BackdropColor(red: channel(r), green: channel(g), blue: channel(b))
    }
}

/// Tone-adjusted corner colours in 8-bit units: whole numbers at rest,
/// fractional mid-crossfade.
struct BackdropFieldColors: Equatable {
    var topLeft: SIMD3<Double>
    var topRight: SIMD3<Double>
    var bottomLeft: SIMD3<Double>
    var bottomRight: SIMD3<Double>

    init(topLeft: SIMD3<Double>, topRight: SIMD3<Double>, bottomLeft: SIMD3<Double>, bottomRight: SIMD3<Double>) {
        self.topLeft = topLeft
        self.topRight = topRight
        self.bottomLeft = bottomLeft
        self.bottomRight = bottomRight
    }

    /// `corners` after `tone`'s adjustment.
    init(_ corners: BackdropCorners, tone: BackdropTone) {
        func units(_ c: BackdropColor) -> SIMD3<Double> {
            let t = tone.adjusted(c)
            return SIMD3(Double(t.red), Double(t.green), Double(t.blue))
        }
        self.init(topLeft: units(corners.topLeft), topRight: units(corners.topRight),
                  bottomLeft: units(corners.bottomLeft), bottomRight: units(corners.bottomRight))
    }

    /// `k` of the way from these colours to `other`, as `from + (to − from)·k`
    /// (ramus `ui/src/lib/ultraBlurField.ts`, `UltraBlurRenderer.tick`).
    func mixed(to other: BackdropFieldColors, _ k: Double) -> BackdropFieldColors {
        func mix(_ a: SIMD3<Double>, _ b: SIMD3<Double>) -> SIMD3<Double> { a + (b - a) * k }
        return BackdropFieldColors(
            topLeft: mix(topLeft, other.topLeft), topRight: mix(topRight, other.topRight),
            bottomLeft: mix(bottomLeft, other.bottomLeft), bottomRight: mix(bottomRight, other.bottomRight))
    }
}

/// The backdrop field: four corner colours, each fading out radially from
/// its corner over a near-black base, blended in gamma-encoded sRGB as CSS
/// gradients are (ramus `ui/src/lib/ultraBlurField.ts`). `backdropShaderSource`
/// (`Shaders/BackdropShaderSource.swift`) draws it; `color(at:_:)` and
/// `pixel(...)` do the same arithmetic on the CPU for tests.
///
/// The layers paint back to front: bottom left, bottom right, top right,
/// top left. Paint order matters: the top-left corner dominates wherever it
/// is opaque, and the others show through where it has faded. ramus's tone
/// values were tuned against this order.
enum BackdropField {
    /// Colour under the corner layers, 8-bit sRGB (`FIELD_BASE`).
    static let base = SIMD3<Double>(5, 5, 8)
    /// Grey level of the page under the dimmed field: ramus's `html`
    /// background, `#111` (`ui/src/styles.css`).
    static let page = 17.0
    /// Corner fade as (distance, coverage) stops (`FALLOFF_STOPS`). Distance
    /// is the fraction along the ray of a CSS `ellipse farthest-corner`
    /// gradient centred on the corner. Each corner has faded out by 80%, so
    /// no layer covers the whole surface. The intermediate stops ease the
    /// fade, where a plain linear fade would end in a visible arc.
    static let falloffStops: [(Double, Double)] = [(0, 1), (0.4, 0.55), (0.62, 0.18), (0.8, 0)]
    /// Colour crossfade when the corners change (`TRANSITION_MS`).
    static let transitionSeconds = 0.8

    /// Coverage of a corner's colour at ray fraction `t`: piecewise linear
    /// through `falloffStops`, 0 beyond the last stop.
    static func falloff(_ t: Double) -> Double {
        for i in 1..<falloffStops.count {
            let (p0, a0) = falloffStops[i - 1]
            let (p1, a1) = falloffStops[i]
            if t < p1 { return a0 * (1 - (t - p0) / (p1 - p0)) + a1 * ((t - p0) / (p1 - p0)) }
        }
        return falloffStops[falloffStops.count - 1].1
    }

    /// CSS `ease-in-out`, `cubic-bezier(0.42, 0, 0.58, 1)`: the curve's x(t)
    /// solved for `x` by bisection, then y(t) (ramus
    /// `ui/src/lib/ultraBlurField.ts`, `easeInOut`).
    static func easeInOut(_ x: Double) -> Double {
        if x <= 0 { return 0 }
        if x >= 1 { return 1 }
        func bez(_ t: Double, _ p1: Double, _ p2: Double) -> Double {
            3 * (1 - t) * (1 - t) * t * p1 + 3 * (1 - t) * t * t * p2 + t * t * t
        }
        var lo = 0.0
        var hi = 1.0
        var t = x
        for _ in 0..<24 {
            t = (lo + hi) / 2
            if bez(t, 0.42, 0.58) < x { lo = t } else { hi = t }
        }
        return bez(t, 0, 1)
    }

    /// The field at `uv` (0...1 across and down), 0...1 per channel, before
    /// the dim. For a gradient centred on a corner of a W × H box,
    /// `ellipse farthest-corner` has radii √2·W and √2·H, so the ray
    /// fraction at `uv` is |uv − corner| / √2.
    static func color(at uv: SIMD2<Double>, _ colors: BackdropFieldColors) -> SIMD3<Double> {
        let layers: [(SIMD3<Double>, SIMD2<Double>)] = [
            (colors.bottomLeft, SIMD2(0, 1)), (colors.bottomRight, SIMD2(1, 1)),
            (colors.topRight, SIMD2(1, 0)), (colors.topLeft, SIMD2(0, 0)),
        ]
        var c = base / 255
        for (color, corner) in layers {
            let a = falloff(simd_length(uv - corner) * 0.70710678)
            c = color / 255 * a + c * (1 - a)
        }
        return c
    }

    /// Pixel (`x`, `y`) of a `width` × `height` image as the shader writes it
    /// before the dither, 0...1 per channel: the field at the pixel's centre,
    /// dimmed to `tone.opacity` over the page colour, times `strength`.
    static func pixel(x: Int, y: Int, width: Int, height: Int, _ colors: BackdropFieldColors,
                      tone: BackdropTone, strength: Double) -> SIMD3<Double> {
        let uv = SIMD2((Double(x) + 0.5) / Double(width), (Double(y) + 0.5) / Double(height))
        let c = color(at: uv, colors)
        return (c * tone.opacity + SIMD3(repeating: page / 255 * (1 - tone.opacity))) * strength
    }
}

/// The backdrop shader's inputs. The layout matches `BackdropUniforms` in
/// `backdropShaderSource` (`Shaders/BackdropShaderSource.swift`).
struct BackdropUniforms: Equatable {
    /// Corner colours in paint order, 0...1; `w` is unused.
    var bottomLeft: SIMD4<Float>
    var bottomRight: SIMD4<Float>
    var topRight: SIMD4<Float>
    var topLeft: SIMD4<Float>
    /// `rgb`: `BackdropField.base`; `w`: `BackdropField.page`; all 0...1.
    var base: SIMD4<Float>
    /// The pass's target size in pixels; the renderer fills it in.
    var viewport: SIMD2<Float>
    var opacity: Float
    /// 1 draws the field; 0 draws black.
    var strength: Float

    init(colors: BackdropFieldColors, tone: BackdropTone, strength: Double) {
        func unit(_ c: SIMD3<Double>) -> SIMD4<Float> {
            SIMD4(Float(c.x / 255), Float(c.y / 255), Float(c.z / 255), 0)
        }
        bottomLeft = unit(colors.bottomLeft)
        bottomRight = unit(colors.bottomRight)
        topRight = unit(colors.topRight)
        topLeft = unit(colors.topLeft)
        let b = BackdropField.base / 255
        base = SIMD4(Float(b.x), Float(b.y), Float(b.z), Float(BackdropField.page / 255))
        viewport = .zero
        opacity = Float(tone.opacity)
        self.strength = Float(strength)
    }
}
