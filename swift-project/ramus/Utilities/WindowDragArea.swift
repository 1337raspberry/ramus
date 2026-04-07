import SwiftUI
import AppKit

/// Attaches a double-click-to-zoom handler to the window's title bar area.
final class WindowDoubleClickHandler: NSObject {
    private weak var window: NSWindow?
    private var monitor: Any?

    static func attach(to window: NSWindow) {
        // Avoid duplicate handlers
        let key = "com.raspsoft.ramus.doubleClickHandler"
        guard objc_getAssociatedObject(window, key) == nil else { return }

        let handler = WindowDoubleClickHandler()
        handler.window = window
        handler.monitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseUp) { event in
            if event.clickCount == 2, let w = handler.window {
                let locationInWindow = event.locationInWindow
                let contentHeight = w.contentLayoutRect.height
                let titleBarHeight = w.frame.height - contentHeight
                let windowHeight = w.frame.height
                // Click is in title bar area if y > contentHeight (title bar is at top)
                if locationInWindow.y > windowHeight - titleBarHeight {
                    w.zoom(nil)
                }
            }
            return event
        }

        objc_setAssociatedObject(window, key, handler, .OBJC_ASSOCIATION_RETAIN)
    }

    deinit {
        if let monitor { NSEvent.removeMonitor(monitor) }
    }
}

/// View modifier to access the hosting NSWindow.
struct NSWindowAccessor: ViewModifier {
    let callback: (NSWindow) -> Void

    func body(content: Content) -> some View {
        content.background(NSWindowReader(callback: callback))
    }
}

private struct NSWindowReader: NSViewRepresentable {
    let callback: (NSWindow) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        DispatchQueue.main.async {
            if let window = view.window {
                callback(window)
            }
        }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}
}

extension View {
    func onNSWindow(_ callback: @escaping (NSWindow) -> Void) -> some View {
        modifier(NSWindowAccessor(callback: callback))
    }
}

