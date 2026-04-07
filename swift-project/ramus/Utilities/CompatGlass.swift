import SwiftUI

extension View {
    @ViewBuilder
    func compatGlassRegular(in shape: some Shape) -> some View {
        if #available(macOS 26, *) {
            self.glassEffect(.regular, in: shape)
        } else {
            self.background(.ultraThinMaterial, in: shape)
        }
    }

    @ViewBuilder
    func compatGlassClear(in shape: some Shape) -> some View {
        if #available(macOS 26, *) {
            self.glassEffect(.clear, in: shape)
        } else {
            self.background(.ultraThinMaterial, in: shape)
        }
    }
}
