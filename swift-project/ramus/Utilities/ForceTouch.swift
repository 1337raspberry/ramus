import SwiftUI
import AppKit

/// Detects force touch (deep press) on a Force Touch trackpad.
/// Fires callback with haptic feedback when pressure reaches stage 2.
/// Uses a local event monitor so SwiftUI gestures (tap, double-tap) are not blocked.
struct ForceTouchModifier: ViewModifier {

    var action: () -> Void

    func body(content: Content) -> some View {
        content.background {
            ForceTouchRepresentable(action: action)
        }
    }
}

extension View {
    func onForceTouch(perform action: @escaping () -> Void) -> some View {
        modifier(ForceTouchModifier(action: action))
    }
}

// MARK: - NSViewRepresentable

private struct ForceTouchRepresentable: NSViewRepresentable {

    var action: () -> Void

    func makeNSView(context: Context) -> ForceTouchNSView {
        let view = ForceTouchNSView()
        view.action = action
        return view
    }

    func updateNSView(_ nsView: ForceTouchNSView, context: Context) {
        nsView.action = action
    }
}

/// Transparent NSView that monitors pressure events via a local event monitor.
/// Returns nil from hitTest so all normal mouse events pass through to SwiftUI.
private final class ForceTouchNSView: NSView {

    var action: (() -> Void)?
    private var monitor: Any?
    private var didFire = false

    // Let all normal mouse events pass through to SwiftUI underneath
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil && monitor == nil {
            monitor = NSEvent.addLocalMonitorForEvents(matching: .pressure) { [weak self] event in
                self?.handlePressure(event)
                return event
            }
        } else if window == nil, let m = monitor {
            NSEvent.removeMonitor(m)
            monitor = nil
        }
    }

    private func handlePressure(_ event: NSEvent) {
        guard let window, event.window === window else { return }

        let loc = convert(event.locationInWindow, from: nil)
        guard bounds.contains(loc) else { return }

        if event.stage == 2, !didFire {
            didFire = true
            NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .now)
            action?()
        } else if event.stage <= 0 {
            didFire = false
        }
    }

    deinit {
        if let m = monitor { NSEvent.removeMonitor(m) }
    }
}
