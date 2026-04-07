import SwiftUI
import AppKit

/// A three-column layout with draggable dividers, replacing NavigationSplitView
/// to avoid the sidebar container chrome. Each column floats directly over
/// the window background with no additional material layers.
struct ThreeColumnLayout<Sidebar: View, Content: View, Detail: View>: View {

    @ViewBuilder var sidebar: () -> Sidebar
    @ViewBuilder var content: () -> Content
    @ViewBuilder var detail: () -> Detail

    // Persisted column widths — nil means "use default proportion"
    @State private var sidebarWidth: CGFloat?
    @State private var detailWidth: CGFloat?

    private let sidebarMin: CGFloat = 180
    private let sidebarMax: CGFloat = 350
    private let sidebarDefault: CGFloat = 220

    private let detailMin: CGFloat = 280
    private let detailMax: CGFloat = 800
    private let detailDefault: CGFloat = 420

    private let contentMin: CGFloat = 200
    private let dividerWidth: CGFloat = 9

    var body: some View {
        GeometryReader { geo in
            let totalWidth = geo.size.width
            let sw = sidebarWidth ?? sidebarDefault
            let dw = detailWidth ?? detailDefault
            // Content gets whatever remains
            let cw = max(contentMin, totalWidth - sw - dw - dividerWidth * 2)

            HStack(spacing: 0) {
                sidebar()
                    .frame(width: sw, height: geo.size.height)

                DragDivider(visible: false, onDrag: { delta in
                    let newSidebar = (sidebarWidth ?? sidebarDefault) + delta
                    sidebarWidth = clamp(newSidebar, min: sidebarMin, max: min(sidebarMax, totalWidth - (detailWidth ?? detailDefault) - contentMin - dividerWidth * 2))
                })

                content()
                    .frame(width: cw, height: geo.size.height)

                DragDivider { delta in
                    // Dragging right makes detail smaller
                    let newDetail = (detailWidth ?? detailDefault) - delta
                    detailWidth = clamp(newDetail, min: detailMin, max: min(detailMax, totalWidth - (sidebarWidth ?? sidebarDefault) - contentMin - dividerWidth * 2))
                }

                detail()
                    .frame(width: dw, height: geo.size.height)
            }
        }
    }

    private func clamp(_ value: CGFloat, min: CGFloat, max: CGFloat) -> CGFloat {
        Swift.min(Swift.max(value, min), max)
    }
}

/// A thin draggable divider between columns.
/// Uses a NonDraggableView underlay so `isMovableByWindowBackground` doesn't
/// swallow the resize gesture.
private struct DragDivider: View {
    var visible: Bool = true
    var onDrag: (CGFloat) -> Void

    var body: some View {
        ZStack {
            Color.clear
            Rectangle()
                .fill(visible ? Color.primary.opacity(0.15) : Color.clear)
                .frame(width: 1)
        }
        .frame(width: 9)
        .overlay(ResizeCursorArea())
        .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 1)
                    .onChanged { value in
                        onDrag(value.translation.width)
                    }
            )
    }
}

/// An NSView that sets the resize cursor via a tracking area and
/// prevents isMovableByWindowBackground from stealing drags.
private struct ResizeCursorArea: NSViewRepresentable {
    func makeNSView(context: Context) -> ResizeCursorNSView { ResizeCursorNSView() }
    func updateNSView(_ nsView: ResizeCursorNSView, context: Context) {}
}

private final class ResizeCursorNSView: NSView {
    override var mouseDownCanMoveWindow: Bool { false }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for area in trackingAreas { removeTrackingArea(area) }
        addTrackingArea(NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .activeInActiveApp, .inVisibleRect, .cursorUpdate],
            owner: self
        ))
    }

    override func cursorUpdate(with event: NSEvent) {
        NSCursor.resizeLeftRight.set()
    }
}
