import RamusWidgetCore
import SwiftUI

extension Color {
    /// A six-digit hex colour, with or without a leading `#`.
    init(hex: String) {
        let h = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
        let v = UInt32(h, radix: 16) ?? 0
        self.init(
            red: Double((v >> 16) & 0xff) / 255,
            green: Double((v >> 8) & 0xff) / 255,
            blue: Double(v & 0xff) / 255
        )
    }
}

/// Four gradient corners, top-left, top-right, bottom-right, bottom-left.
struct Corners {
    let topLeft: Color
    let topRight: Color
    let bottomRight: Color
    let bottomLeft: Color

    /// The app's launch gradient.
    static let pinkySunset = Corners(hex: ["853e43", "875147", "854256", "853646"])

    init(hex: [String]) {
        topLeft = Color(hex: hex[0])
        topRight = Color(hex: hex[1])
        bottomRight = Color(hex: hex[2])
        bottomLeft = Color(hex: hex[3])
    }

    init(_ colors: WidgetSnapshot.Colors?) {
        guard let c = colors else {
            self = .pinkySunset
            return
        }
        self.init(hex: [c.topLeft, c.topRight, c.bottomRight, c.bottomLeft])
    }
}

extension Color {
    func mix(_ other: Color) -> Color {
        let a = UIColor(self), b = UIColor(other)
        var (r1, g1, b1, a1): (CGFloat, CGFloat, CGFloat, CGFloat) = (0, 0, 0, 0)
        var (r2, g2, b2, a2): (CGFloat, CGFloat, CGFloat, CGFloat) = (0, 0, 0, 0)
        a.getRed(&r1, green: &g1, blue: &b1, alpha: &a1)
        b.getRed(&r2, green: &g2, blue: &b2, alpha: &a2)
        return Color(red: (r1 + r2) / 2, green: (g1 + g2) / 2, blue: (b1 + b2) / 2)
    }
}

/// The four-corner blur background the app paints behind its player.
struct CornerGradient: View {
    let corners: Corners

    var body: some View {
        if #available(iOS 18.0, *) {
            MeshGradient(
                width: 2, height: 2,
                points: [[0, 0], [1, 0], [0, 1], [1, 1]],
                colors: [corners.topLeft, corners.topRight, corners.bottomLeft, corners.bottomRight]
            )
        } else {
            ZStack {
                corners.topLeft.mix(corners.bottomRight)
                RadialGradient(colors: [corners.topLeft, .clear], center: .topLeading, startRadius: 0, endRadius: 260)
                RadialGradient(colors: [corners.topRight, .clear], center: .topTrailing, startRadius: 0, endRadius: 260)
                RadialGradient(colors: [corners.bottomRight, .clear], center: .bottomTrailing, startRadius: 0, endRadius: 260)
                RadialGradient(colors: [corners.bottomLeft, .clear], center: .bottomLeading, startRadius: 0, endRadius: 260)
            }
        }
    }
}

/// A translucent rounded tile, as used for the idle actions.
struct TileBackground: View {
    var body: some View {
        RoundedRectangle(cornerRadius: 16, style: .continuous)
            .fill(.white.opacity(0.12))
            .overlay(
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .strokeBorder(.white.opacity(0.14), lineWidth: 0.5)
            )
    }
}

struct RamusLogo: View {
    var body: some View {
        Image("RamusR")
            .resizable()
            .renderingMode(.template)
            .foregroundStyle(.white)
            .frame(width: 18, height: 18)
            .accessibilityHidden(true)
    }
}
