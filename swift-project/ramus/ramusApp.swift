import SwiftUI
import AppKit
import Nuke
import PlexAPI

/// Keeps the app running when the window is closed (red traffic light hides, dock click re-shows).
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Intercept the main window's close button so it hides instead of destroying
        if let window = NSApplication.shared.windows.first {
            window.delegate = self
        }
        cleanUpMenuBar()
    }

    /// Strip menus that don't apply to a music player.
    private func cleanUpMenuBar() {
        guard let mainMenu = NSApplication.shared.mainMenu else { return }
        // Remove View and Help menus entirely
        for title in ["View", "Help"] {
            if let item = mainMenu.items.first(where: { $0.title == title }) {
                mainMenu.removeItem(item)
            }
        }
        // Trim Window menu: keep only Minimize and Bring All to Front
        if let windowItem = mainMenu.items.first(where: { $0.title == "Window" }),
           let submenu = windowItem.submenu {
            let keep: Set<String> = ["performMiniaturize:", "arrangeInFront:"]
            for item in submenu.items where item.action.map({ !keep.contains(NSStringFromSelector($0)) }) ?? true {
                submenu.removeItem(item)
            }
        }
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        sender.orderOut(nil) // hide, don't close — keeps SwiftUI state alive
        return false
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag, let window = sender.windows.first {
            window.makeKeyAndOrderFront(nil)
            return false
        }
        return true
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }
}

@main
struct RamusApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    init() {
        // Hide all scrollbars app-wide via swizzle
        HiddenScrollbars.install()
        // Disable window tabbing (removes "Show Tab Bar" / "Show All Tabs" from View menu)
        NSWindow.allowsAutomaticWindowTabbing = false
        // Persistent 500 MB image cache (LRU, ignores HTTP headers)
        ImagePipeline.shared = ImagePipeline(
            configuration: .withDataCache(
                name: "com.raspsoft.ramus.ImageCache",
                sizeLimit: 500 * 1024 * 1024
            ),
            delegate: PlexCacheDelegate()
        )
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
                .preferredColorScheme(.dark)
        }
        .windowStyle(.hiddenTitleBar)
        .defaultSize(width: 1000, height: 700)
        .commands {
            CommandGroup(replacing: .appInfo) {
                Button("About ramus") {
                    NotificationCenter.default.post(name: .openAbout, object: nil)
                }
            }
            CommandGroup(replacing: .newItem) { }
            CommandGroup(replacing: .pasteboard) { }
            CommandGroup(replacing: .undoRedo) { }
            CommandGroup(replacing: .textEditing) { }
            CommandGroup(after: .appSettings) {
                Button("Settings...") {
                    NotificationCenter.default.post(name: .openSettings, object: nil)
                }
                .keyboardShortcut(",", modifiers: .command)
            }
        }
    }
}

extension Notification.Name {
    static let openSettings = Notification.Name("openSettings")
    static let openAbout = Notification.Name("openAbout")
}

enum HiddenScrollbars {
    static func install() {
        let original = class_getInstanceMethod(NSScrollView.self, #selector(NSScrollView.tile))!
        let swizzled = class_getInstanceMethod(NSScrollView.self, #selector(NSScrollView.hiddenTile))!
        method_exchangeImplementations(original, swizzled)
    }
}

/// Strips the Plex auth token from image URLs so the cache key is just thumb+size.
/// Token rotation (reconnect, re-auth) won't orphan cached images.
/// @unchecked Sendable: ImagePipelineDelegate requires Sendable, but this class
/// is stateless — the single method is a pure function (URL → String). No mutable
/// state to protect.
final class PlexCacheDelegate: ImagePipelineDelegate, @unchecked Sendable {
    func cacheKey(for request: ImageRequest, pipeline: ImagePipeline) -> String? {
        guard let url = request.url,
              var components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return nil
        }
        // Strip token from outer query params
        components.queryItems = components.queryItems?.filter { $0.name != "X-Plex-Token" }
        // Strip token from the inner "url" param (the thumb URL Plex transcodes)
        if let idx = components.queryItems?.firstIndex(where: { $0.name == "url" }),
           let inner = components.queryItems?[idx].value,
           var innerComponents = URLComponents(string: inner) {
            innerComponents.queryItems = innerComponents.queryItems?.filter { $0.name != "X-Plex-Token" }
            components.queryItems?[idx].value = innerComponents.string
        }
        return components.string
    }
}

private var hiddenTileReentrantKey: UInt8 = 0

extension NSScrollView {
    @objc func hiddenTile() {
        // Guard against re-entrancy: setting scrollerStyle triggers tile(),
        // which is swizzled to hiddenTile(). Without this flag the recursion
        // depends on undocumented AppKit ivar-update ordering.
        guard objc_getAssociatedObject(self, &hiddenTileReentrantKey) == nil else {
            hiddenTile() // calls original (swizzled)
            return
        }
        objc_setAssociatedObject(self, &hiddenTileReentrantKey, true, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
        // Force overlay style BEFORE layout — overlay scrollers don't reserve space.
        // Without this, legacy style (or "Always show scrollbars" system pref)
        // subtracts ~17pt from the content view width.
        if scrollerStyle != .overlay {
            scrollerStyle = .overlay
        }
        objc_setAssociatedObject(self, &hiddenTileReentrantKey, nil, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
        hiddenTile() // calls original (swizzled)
        verticalScroller?.alphaValue = 0
        horizontalScroller?.alphaValue = 0
    }
}
