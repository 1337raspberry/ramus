// Ported from ramusTV `RamusTV/Backdrop/BackdropColor.swift`.

import Foundation

/// An 8-bit sRGB colour. Its hex form is six lowercase digits without a
/// `#`, as ramus passes corner colours around.
struct BackdropColor: Equatable {
    var red: UInt8
    var green: UInt8
    var blue: UInt8

    init(red: UInt8, green: UInt8, blue: UInt8) {
        self.red = red
        self.green = green
        self.blue = blue
    }

    init?(hex: String) {
        guard hex.count == 6, hex.allSatisfy(\.isHexDigit), let value = UInt32(hex, radix: 16) else { return nil }
        self.init(red: UInt8(value >> 16), green: UInt8((value >> 8) & 0xff), blue: UInt8(value & 0xff))
    }

    var hex: String { String(format: "%02x%02x%02x", red, green, blue) }
}

/// The backdrop's four corner colours.
struct BackdropCorners: Equatable {
    var topLeft: BackdropColor
    var topRight: BackdropColor
    var bottomLeft: BackdropColor
    var bottomRight: BackdropColor

    /// The brand corners, shown when there is no art: the brand pinks
    /// scaled to about 55% so the field stays comfortable across the whole
    /// screen (ramus `ui/src/lib/accent.ts`, `DEFAULT_BLUR_COLORS`).
    static let brandDefault = BackdropCorners(
        topLeft: BackdropColor(red: 0x85, green: 0x3e, blue: 0x43),
        topRight: BackdropColor(red: 0x87, green: 0x51, blue: 0x47),
        bottomLeft: BackdropColor(red: 0x85, green: 0x36, blue: 0x46),
        bottomRight: BackdropColor(red: 0x85, green: 0x42, blue: 0x56))
}
