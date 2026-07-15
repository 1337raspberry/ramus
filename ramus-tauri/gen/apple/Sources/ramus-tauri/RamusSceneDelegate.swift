import UIKit

// Bridges Tauri/tao into the iOS UIScene lifecycle.
//
// tao (0.34.x) predates scene lifecycle and does two things that break under it:
//   1. It drives its event loop from the classic app-delegate callbacks
//      (`applicationDidBecomeActive:` → `Event::Resumed`, etc.). UIKit stops
//      delivering those to the app delegate once an app adopts scenes, sending
//      the `scene*` equivalents instead — so we forward them along to keep tao's
//      loop pumping.
//   2. It creates its `UIWindow` from the app delegate and never assigns a
//      `windowScene`. A scene-less window is invisible, and — crucially — is
//      absent from `UIApplication.windows`, so the app side can't find it by
//      scanning. Instead the Rust side (which owns the window) hands us the
//      `UIWindow*` via `ramus_ios_main_window()` and we adopt it into the scene.
//
// The manifest that names this class is in project.yml (UIApplicationSceneManifest).
// The proper long-term fix is upstream tao/wry scene support (see Tauri #15719).
@objc(RamusSceneDelegate)
final class RamusSceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?
    private weak var currentWindowScene: UIWindowScene?

    func scene(
        _ scene: UIScene,
        willConnectTo session: UISceneSession,
        options connectionOptions: UIScene.ConnectionOptions
    ) {
        currentWindowScene = scene as? UIWindowScene
        adoptTaoWindow(attempt: 0)
    }

    func sceneDidBecomeActive(_ scene: UIScene) {
        currentWindowScene = scene as? UIWindowScene
        // Keep tao's classic loop pumping (it never receives these under scenes).
        forwardToAppDelegate("applicationDidBecomeActive:")
        adoptTaoWindow(attempt: 0)
    }

    func sceneWillResignActive(_ scene: UIScene) {
        forwardToAppDelegate("applicationWillResignActive:")
    }

    func sceneWillEnterForeground(_ scene: UIScene) {
        forwardToAppDelegate("applicationWillEnterForeground:")
    }

    func sceneDidEnterBackground(_ scene: UIScene) {
        forwardToAppDelegate("applicationDidEnterBackground:")
    }

    /// Invoke a classic app-delegate lifecycle method on tao's app delegate.
    private func forwardToAppDelegate(_ selectorName: String) {
        guard let delegate = UIApplication.shared.delegate as? NSObject else { return }
        let selector = NSSelectorFromString(selectorName)
        guard delegate.responds(to: selector) else { return }
        delegate.perform(selector, with: UIApplication.shared)
    }

    /// Adopt tao's `UIWindow` (fetched from Rust) into our connected scene so it
    /// renders. The window is created during Tauri setup, which runs at launch —
    /// so it's usually ready by the first scene callback — but we retry a few
    /// times in case the scene connects before setup finishes wiring it up.
    private func adoptTaoWindow(attempt: Int) {
        guard let windowScene = currentWindowScene else { return }

        if let raw = ramus_ios_main_window() {
            let window = Unmanaged<UIWindow>.fromOpaque(raw).takeUnretainedValue()
            if window.windowScene !== windowScene {
                window.windowScene = windowScene
            }
            window.makeKeyAndVisible()
            self.window = window
            NSLog("RAMUS_SCENE: adopted \(type(of: window)) into scene frame=\(NSCoder.string(for: window.frame))")
            return
        }

        if attempt < 80 {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self] in
                self?.adoptTaoWindow(attempt: attempt + 1)
            }
        } else {
            NSLog("RAMUS_SCENE: Rust returned no UIWindow after retries")
        }
    }
}
