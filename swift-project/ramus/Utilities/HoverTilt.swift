import SwiftUI

extension View {
    /// Applies a 3D tilt-and-lift effect on mouse hover.
    func hoverTilt(
        maxAngle: Double = 8,
        liftScale: CGFloat = 1.03,
        shadowRadius: CGFloat = 12
    ) -> some View {
        modifier(HoverTiltModifier(
            maxAngle: maxAngle,
            liftScale: liftScale,
            shadowRadius: shadowRadius
        ))
    }
}

private struct HoverTiltModifier: ViewModifier {

    var maxAngle: Double
    var liftScale: CGFloat
    var shadowRadius: CGFloat

    @State private var isHovering = false
    @State private var tiltX: Double = 0
    @State private var tiltY: Double = 0
    @State private var viewSize: CGSize = .zero

    func body(content: Content) -> some View {
        content
            .background(
                GeometryReader { geo in
                    Color.clear
                        .onAppear { viewSize = geo.size }
                        .onChange(of: geo.size) { _, newSize in viewSize = newSize }
                }
            )
            .scaleEffect(isHovering ? liftScale : 1.0)
            .rotation3DEffect(
                .degrees(tiltX),
                axis: (x: 1, y: 0, z: 0),
                perspective: 0.5
            )
            .rotation3DEffect(
                .degrees(tiltY),
                axis: (x: 0, y: 1, z: 0),
                perspective: 0.5
            )
            .shadow(
                color: .black.opacity(isHovering ? 0.3 : 0.0),
                radius: isHovering ? shadowRadius : 0,
                y: isHovering ? shadowRadius / 2 : 0
            )
            .animation(.spring(response: 0.3, dampingFraction: 0.7), value: isHovering)
            .animation(.spring(response: 0.2, dampingFraction: 0.7), value: tiltX)
            .animation(.spring(response: 0.2, dampingFraction: 0.7), value: tiltY)
            .onContinuousHover { phase in
                switch phase {
                case .active(let location):
                    isHovering = true
                    guard viewSize.width > 0, viewSize.height > 0 else { return }
                    let normalizedX = (location.x / viewSize.width - 0.5) * 2
                    let normalizedY = (location.y / viewSize.height - 0.5) * 2
                    tiltX = -normalizedY * maxAngle
                    tiltY = normalizedX * maxAngle
                case .ended:
                    isHovering = false
                    tiltX = 0
                    tiltY = 0
                }
            }
    }
}
