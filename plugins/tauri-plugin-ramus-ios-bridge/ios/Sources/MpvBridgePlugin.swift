import AVFoundation
import MediaPlayer
import Tauri
import UIKit
import WebKit

/// Main plugin class — registered with Tauri via the `@_cdecl` init at
/// the bottom of the file. Each `@objc` method here matches a Rust-side
/// call via `run_mobile_plugin("methodName", args)`.
///
/// The plugin owns three collaborators:
///   - `MpvController` — wraps libmpv for audio-only playback.
///   - `NowPlayingBridge` — `MPNowPlayingInfoCenter` +
///     `MPRemoteCommandCenter` wiring.
///   - implicit `AVAudioSession` — configured once on `initAudio`.
///
/// mpv events fire from the controller on a background queue; the plugin
/// forwards them to Rust via `trigger(name, data:)`, which the Rust side
/// listens to with `app.listen("plugin:ramus-ios-bridge://<name>", …)`.
class MpvBridgePlugin: Plugin {
    private var mpv: MpvController?
    private var nowPlaying: NowPlayingBridge?
    private weak var webView: WKWebView?
    private var searchBar: UISearchBar?
    private var allowEndEditing = false
    private var interruptionObserver: NSObjectProtocol?

    override func load(webview: WKWebView) {
        self.webView = webview
        webview.scrollView.keyboardDismissMode = .interactive
        webview.overrideUserInterfaceStyle = .dark
        Self.removeInputAccessoryView()
    }

    /// Swizzle WKContentView's inputAccessoryView to return nil, removing
    /// the chevron/checkmark toolbar that WKWebView adds above the keyboard.
    private static var swizzled = false
    private static func removeInputAccessoryView() {
        guard !swizzled else { return }
        swizzled = true
        guard let wkContentView = NSClassFromString("WKContentView"),
              let original = class_getInstanceMethod(wkContentView, #selector(getter: UIResponder.inputAccessoryView)),
              let replacement = class_getInstanceMethod(UIView.self, #selector(UIView._ramus_nilAccessoryView))
        else { return }
        method_exchangeImplementations(original, replacement)
    }

    // MARK: - Initialization

    @objc public func initAudio(_ invoke: Invoke) throws {
        let session = AVAudioSession.sharedInstance()
        do {
            try session.setCategory(.playback, mode: .default, options: [])
            try session.setActive(true)
        } catch {
            invoke.reject("failed to activate audio session: \(error)")
            return
        }

        DispatchQueue.main.async {
            UIApplication.shared.beginReceivingRemoteControlEvents()
        }

        if interruptionObserver != nil { return }
        interruptionObserver = NotificationCenter.default.addObserver(
            forName: AVAudioSession.interruptionNotification,
            object: session,
            queue: .main
        ) { [weak self] note in
            guard let info = note.userInfo,
                  let typeVal = info[AVAudioSessionInterruptionTypeKey] as? UInt,
                  let type = AVAudioSession.InterruptionType(rawValue: typeVal)
            else { return }
            switch type {
            case .began:
                self?.mpv?.setPause(true)
            case .ended:
                let opts = info[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0
                if AVAudioSession.InterruptionOptions(rawValue: opts).contains(.shouldResume) {
                    try? session.setActive(true)
                    self?.mpv?.setPause(false)
                }
            @unknown default: break
            }
        }

        if nowPlaying == nil {
            nowPlaying = NowPlayingBridge { [weak self] name, data in
                DispatchQueue.main.async { self?.trigger(name, data: data) }
            }
        }

        invoke.resolve([:])
    }

    @objc public func mpvInit(_ invoke: Invoke) throws {
        if mpv == nil {
            let controller = MpvController()
            guard controller.isReady else {
                invoke.reject("mpv initialization failed")
                return
            }
            controller.onPositionChange = { [weak self] pos in
                DispatchQueue.main.async { self?.trigger("mpvPositionChange", data: ["position": pos]) }
            }
            controller.onDurationChange = { [weak self] dur in
                DispatchQueue.main.async { self?.trigger("mpvDurationChange", data: ["duration": dur]) }
            }
            controller.onPlaylistPosChange = { [weak self] pos in
                DispatchQueue.main.async { self?.trigger("mpvPlaylistPosChange", data: ["index": pos]) }
            }
            controller.onPauseChange = { [weak self] paused in
                DispatchQueue.main.async { self?.trigger("mpvPauseChange", data: ["paused": paused]) }
            }
            controller.onIdleActive = { [weak self] in
                DispatchQueue.main.async { self?.trigger("mpvIdleActive", data: [:]) }
            }
            controller.onFileLoaded = { [weak self] in
                DispatchQueue.main.async { self?.trigger("mpvFileLoaded", data: [:]) }
            }
            controller.onFileEnded = { [weak self] reason in
                DispatchQueue.main.async { self?.trigger("mpvFileEnded", data: ["reason": reason]) }
            }
            mpv = controller
        }
        invoke.resolve([:])
    }

    // MARK: - MPV command proxies

    @objc public func mpvLoadFile(_ invoke: Invoke) throws {
        guard let mpv else {
            invoke.reject("mpv not initialized")
            return
        }
        let args = try invoke.parseArgs(LoadFileArgs.self)
        mpv.loadFile(args.url, mode: args.mode, options: args.options)
        invoke.resolve([:])
    }

    @objc public func mpvLoadFileAt(_ invoke: Invoke) throws {
        guard let mpv else {
            invoke.reject("mpv not initialized")
            return
        }
        let args = try invoke.parseArgs(LoadFileAtArgs.self)
        mpv.loadFileAt(args.url, index: args.index, options: args.options)
        invoke.resolve([:])
    }

    @objc public func mpvPlaylistPlayIndex(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(PlaylistIndexArgs.self)
        mpv?.playlistPlayIndex(args.index)
        invoke.resolve([:])
    }

    @objc public func mpvPlaylistRemove(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(PlaylistIndexArgs.self)
        mpv?.playlistRemove(args.index)
        invoke.resolve([:])
    }

    @objc public func mpvPlaylistMove(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(PlaylistMoveArgs.self)
        mpv?.playlistMove(from: args.from, to: args.to)
        invoke.resolve([:])
    }

    @objc public func mpvSeek(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(SeekArgs.self)
        mpv?.seek(to: args.position)
        invoke.resolve([:])
    }

    @objc public func mpvSetPause(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(PauseArgs.self)
        mpv?.setPause(args.paused)
        invoke.resolve([:])
    }

    @objc public func mpvSetVolume(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(VolumeArgs.self)
        mpv?.setVolume(args.volume)
        invoke.resolve([:])
    }

    @objc public func mpvGetVolume(_ invoke: Invoke) throws {
        let value = mpv?.getVolume() ?? 100.0
        invoke.resolve(["volume": value])
    }

    @objc public func mpvGetEqConfig(_ invoke: Invoke) throws {
        invoke.resolve([
            "frequencies": [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000],
            "minGain": -12.0,
            "maxGain": 12.0
        ])
    }

    @objc public func mpvSetAudioFilters(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(AudioFiltersArgs.self)
        mpv?.setAudioFilters(args.value)
        invoke.resolve([:])
    }

    @objc public func mpvStop(_ invoke: Invoke) throws {
        mpv?.stop()
        invoke.resolve([:])
    }

    // MARK: - Now Playing

    @objc public func nowPlayingUpdate(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(NowPlayingMetadata.self)
        nowPlaying?.update(args)
        invoke.resolve([:])
    }

    @objc public func nowPlayingClear(_ invoke: Invoke) throws {
        nowPlaying?.clear()
        invoke.resolve([:])
    }

    // MARK: - Keyboard

    @objc public func dismissKeyboard(_ invoke: Invoke) throws {
        DispatchQueue.main.async { [weak self] in
            self?.webView?.endEditing(true)
        }
        invoke.resolve([:])
    }

    // MARK: - Native Search Bar

    @objc public func showNativeSearchBar(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(ShowSearchBarArgs.self)
        DispatchQueue.main.async { [weak self] in
            self?.presentSearchBar(initialText: args.initialQuery)
        }
        invoke.resolve([:])
    }

    @objc public func hideNativeSearchBar(_ invoke: Invoke) throws {
        DispatchQueue.main.async { [weak self] in
            self?.removeSearchBar()
        }
        invoke.resolve([:])
    }

    private func presentSearchBar(initialText: String) {
        guard searchBar == nil, let webView = webView, let parent = webView.superview else { return }

        let bar = UISearchBar()
        bar.delegate = self
        bar.text = initialText.isEmpty ? nil : initialText
        bar.placeholder = "Search"
        bar.showsCancelButton = true
        bar.searchBarStyle = .minimal
        bar.overrideUserInterfaceStyle = .dark
        bar.tintColor = .white
        bar.translatesAutoresizingMaskIntoConstraints = false

        parent.addSubview(bar)
        NSLayoutConstraint.activate([
            bar.topAnchor.constraint(equalTo: parent.safeAreaLayoutGuide.topAnchor),
            bar.leadingAnchor.constraint(equalTo: parent.leadingAnchor),
            bar.trailingAnchor.constraint(equalTo: parent.trailingAnchor),
        ])

        searchBar = bar
        bar.becomeFirstResponder()
    }

    private func removeSearchBar() {
        searchBar?.removeFromSuperview()
        searchBar = nil
        webView?.endEditing(true)
    }

    private func dispatchSearchEvent(_ name: String, detail: [String: Any]?) {
        let detailJS: String
        if let d = detail,
           let data = try? JSONSerialization.data(withJSONObject: d),
           let s = String(data: data, encoding: .utf8) {
            detailJS = s
        } else {
            detailJS = "null"
        }
        let js = "window.dispatchEvent(new CustomEvent('\(name)', { detail: \(detailJS) }))"
        webView?.evaluateJavaScript(js, completionHandler: nil)
    }

    // MARK: - Keychain

    @objc public func keychainRead(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(KeychainAccountArgs.self)
        // Resolve with empty string on miss and let the Rust side interpret
        // empty-string as "not present". `JSObject` doesn't accept nil
        // values, so we can't pass `NSNull` without an extra encoding hop.
        let value = KeychainBridge.shared.read(account: args.account) ?? ""
        invoke.resolve(["value": value])
    }

    @objc public func keychainWrite(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(KeychainWriteArgs.self)
        let ok = KeychainBridge.shared.write(account: args.account, value: args.value)
        invoke.resolve(["ok": ok])
    }

    @objc public func keychainDelete(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(KeychainAccountArgs.self)
        let ok = KeychainBridge.shared.delete(account: args.account)
        invoke.resolve(["ok": ok])
    }

    @objc public func excludeFromBackup(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(ExcludeBackupArgs.self)
        var url = URL(fileURLWithPath: args.path)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        do {
            try url.setResourceValues(values)
            invoke.resolve(["ok": true])
        } catch {
            invoke.resolve(["ok": false])
        }
    }
}

// MARK: - Argument payloads

class LoadFileArgs: Decodable {
    let url: String
    let mode: String
    let options: String?
}

class LoadFileAtArgs: Decodable {
    let url: String
    let index: Int
    let options: String?
}

class PlaylistIndexArgs: Decodable {
    let index: Int
}

class PlaylistMoveArgs: Decodable {
    let from: Int
    let to: Int
}

class SeekArgs: Decodable {
    let position: Double
}

class PauseArgs: Decodable {
    let paused: Bool
}

class VolumeArgs: Decodable {
    let volume: Double
}

class AudioFiltersArgs: Decodable {
    let value: String
}

class KeychainAccountArgs: Decodable {
    let account: String
}

class KeychainWriteArgs: Decodable {
    let account: String
    let value: String
}

class ExcludeBackupArgs: Decodable {
    let path: String
}

class ShowSearchBarArgs: Decodable {
    let initialQuery: String
}

class NowPlayingMetadata: Decodable {
    let title: String
    let artist: String
    let album: String
    let duration: Double
    let position: Double
    let isPlaying: Bool
    let coverUrl: String?
}

extension MpvBridgePlugin: UISearchBarDelegate {
    func searchBar(_ searchBar: UISearchBar, textDidChange searchText: String) {
        dispatchSearchEvent("nativeSearchText", detail: ["text": searchText])
    }

    func searchBarShouldEndEditing(_ searchBar: UISearchBar) -> Bool {
        if allowEndEditing { return true }
        // User tapped outside (e.g. on a search result). Yank the bar
        // before UIKit transfers first-responder to the WKWebView.
        removeSearchBar()
        return false
    }

    func searchBarSearchButtonClicked(_ searchBar: UISearchBar) {
        // Flag lets shouldEndEditing return true so resignFirstResponder
        // dismisses the keyboard normally, keeping the bar visible.
        allowEndEditing = true
        searchBar.resignFirstResponder()
        allowEndEditing = false
    }

    func searchBarCancelButtonClicked(_ searchBar: UISearchBar) {
        removeSearchBar()
        dispatchSearchEvent("nativeSearchCancel", detail: nil)
    }
}

extension UIView {
    @objc func _ramus_nilAccessoryView() -> UIView? { nil }
}

@_cdecl("init_plugin_ramus_ios_bridge")
func initPlugin() -> Plugin {
    return MpvBridgePlugin()
}
