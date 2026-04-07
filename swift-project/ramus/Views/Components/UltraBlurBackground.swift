import SwiftUI
import Models

/// Full-window gradient background derived from Plex album UltraBlurColors.
/// Uses MeshGradient for a smooth 4-corner blend similar to Plexamp's ultrablur.
struct UltraBlurBackground: View {

    let colors: UltraBlurColors

    var body: some View {
        let tl = RGB(hex: colors.topLeft).pastelised()
        let tr = RGB(hex: colors.topRight).pastelised()
        let bl = RGB(hex: colors.bottomLeft).pastelised()
        let br = RGB(hex: colors.bottomRight).pastelised()
        let center = tl.mixed(with: tr).mixed(with: bl.mixed(with: br))

        MeshGradient(
            width: 3, height: 3,
            points: [
                [0.0, 0.0], [0.5, 0.0], [1.0, 0.0],
                [0.0, 0.5], [0.5, 0.5], [1.0, 0.5],
                [0.0, 1.0], [0.5, 1.0], [1.0, 1.0],
            ],
            colors: [
                tl.color, tl.mixed(with: tr).color, tr.color,
                tl.mixed(with: bl).color, center.color, tr.mixed(with: br).color,
                bl.color, bl.mixed(with: br).color, br.color,
            ]
        )
        .ignoresSafeArea()
    }
}

// MARK: - RGB Helper

private struct RGB {
    let r: Double
    let g: Double
    let b: Double

    init(r: Double, g: Double, b: Double) {
        self.r = r
        self.g = g
        self.b = b
    }

    init(hex: String) {
        let cleaned = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
        guard cleaned.count == 6,
              let value = UInt64(cleaned, radix: 16) else {
            self.r = 0; self.g = 0; self.b = 0
            return
        }
        self.r = Double((value >> 16) & 0xFF) / 255.0
        self.g = Double((value >> 8) & 0xFF) / 255.0
        self.b = Double(value & 0xFF) / 255.0
    }

    var color: Color {
        Color(red: r, green: g, blue: b)
    }

    func mixed(with other: RGB) -> RGB {
        RGB(
            r: (r + other.r) / 2,
            g: (g + other.g) / 2,
            b: (b + other.b) / 2
        )
    }

    /// Desaturate and darken for a muted background that lets accent colors pop.
    func pastelised() -> RGB {
        let maxC = max(r, g, b), minC = min(r, g, b)
        var l = (maxC + minC) / 2.0
        let delta = maxC - minC

        guard delta > 0.001 else {
            // Achromatic — darken but enforce minimum lightness
            let darkened = max(l * 0.75, 0.12)
            return RGB(r: darkened, g: darkened, b: darkened)
        }

        var s = l < 0.5 ? delta / (maxC + minC) : delta / (2.0 - maxC - minC)

        // Desaturate 35%, darken 25%, enforce minimum lightness
        s *= 0.65
        l = max(l * 0.75, 0.20)

        // HSL → RGB
        var h: Double
        if maxC == r {
            h = (g - b) / delta + (g < b ? 6 : 0)
        } else if maxC == g {
            h = (b - r) / delta + 2
        } else {
            h = (r - g) / delta + 4
        }
        h /= 6.0

        func hue2rgb(_ p: Double, _ q: Double, _ t: Double) -> Double {
            var t = t
            if t < 0 { t += 1 }
            if t > 1 { t -= 1 }
            if t < 1/6.0 { return p + (q - p) * 6 * t }
            if t < 1/2.0 { return q }
            if t < 2/3.0 { return p + (q - p) * (2/3.0 - t) * 6 }
            return p
        }

        let q = l < 0.5 ? l * (1 + s) : l + s - l * s
        let p = 2 * l - q
        return RGB(
            r: hue2rgb(p, q, h + 1/3.0),
            g: hue2rgb(p, q, h),
            b: hue2rgb(p, q, h - 1/3.0)
        )
    }
}

