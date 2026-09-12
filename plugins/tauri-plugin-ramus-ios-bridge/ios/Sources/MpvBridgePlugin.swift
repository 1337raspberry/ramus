import AVFoundation
import MediaPlayer
import Network
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
    /// Row holding the search bar and its Cancel button — the view that is
    /// actually added to the hierarchy.
    private var searchStrip: UIView?
    /// The strip's placement constraints, replaced when the page re-reports
    /// its slot without the bar being torn down.
    private var searchStripPlacement: [NSLayoutConstraint] = []
    private var interruptionObserver: NSObjectProtocol?
    private var pathMonitor: NWPathMonitor?
    private let pathMonitorQueue = DispatchQueue(label: "com.raspsoft.ramus.path-monitor")
    /// Snapshot of the most recent NWPath, polled by `getNetworkInfo`. The
    /// debug panel reads this synchronously to label the current network
    /// type without firing a fresh probe.
    ///
    /// `JSObject` is `[String: any JSValue]` — a Swift `Dictionary` backed
    /// by a class. `handlePathUpdate` writes it on main; `getNetworkInfo`
    /// reads it on whatever thread Tauri's IPC dispatch picks. Both
    /// accesses must go through `lastPathSnapshotLock` to avoid UB.
    private var lastPathSnapshot: JSObject = [:]
    private let lastPathSnapshotLock = NSLock()
    /// Background-task assertion held while playback recovery is in
    /// flight. The `audio` background mode keeps the process alive only
    /// while audio is actually rendering — during a reconnect the app is
    /// silent, so without this assertion iOS suspends it seconds into the
    /// outage and every timer/monitor the recovery relies on freezes.
    /// Accessed on the main queue only.
    private var recoveryGraceTask: UIBackgroundTaskIdentifier = .invalid
    private var keyboardObservers: [NSObjectProtocol] = []
    private var scrollPin: NSKeyValueObservation?

    override func load(webview: WKWebView) {
        self.webView = webview
        webview.scrollView.keyboardDismissMode = .interactive
        webview.overrideUserInterfaceStyle = .dark
        Self.removeInputAccessoryView()
        installKeyboardInsetObserver()
        // All scrolling happens inside the page's own DOM scrollers — the
        // outer scroll view never legitimately moves. UIKit still scrolls
        // it to "reveal" a focused input under the keyboard, which would
        // stack with the page's own --keyboard-inset lift and shove the
        // whole UI off the top. Pin it flat instead.
        scrollPin = webview.scrollView.observe(\.contentOffset, options: [.new]) {
            scrollView, _ in
            if scrollView.contentOffset != .zero {
                scrollView.setContentOffset(.zero, animated: false)
            }
        }
    }

    deinit {
        // NWPathMonitor must be explicitly cancelled before release;
        // letting ARC drop it leaves the dispatch source live and leaks
        // the kernel network-path subscription. Same for the audio
        // session interruption observer and the keyboard observers.
        pathMonitor?.cancel()
        if let token = interruptionObserver {
            NotificationCenter.default.removeObserver(token)
        }
        for token in keyboardObservers {
            NotificationCenter.default.removeObserver(token)
        }
    }

    /// Pushes the software keyboard's overlap with the webview into the
    /// page as a `--keyboard-inset` CSS variable (px). The keyboard slides
    /// OVER the webview — it never resizes it — and WebKit's visualViewport
    /// does not report the occlusion, so bottom-anchored page UI holding a
    /// text input reads this variable to lift itself clear. Same host-push
    /// pattern as the Android activity's `--android-inset-*` variables.
    private func installKeyboardInsetObserver() {
        let center = NotificationCenter.default
        let push: (CGFloat) -> Void = { [weak self] inset in
            let js =
                "document.documentElement.style.setProperty('--keyboard-inset', '\(Int(inset.rounded()))px')"
            self?.webView?.evaluateJavaScript(js, completionHandler: nil)
        }
        keyboardObservers.append(
            center.addObserver(
                forName: UIResponder.keyboardWillChangeFrameNotification, object: nil, queue: .main
            ) { [weak self] note in
                guard let webView = self?.webView,
                    let frameValue = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey]
                        as? NSValue
                else { return }
                // The end frame arrives in screen coordinates; a hide (or a
                // detached hardware-keyboard bar) lands outside the webview,
                // so the intersection naturally reports 0.
                let endFrame = webView.convert(frameValue.cgRectValue, from: nil)
                let overlap = webView.bounds.intersection(endFrame)
                push(overlap.isNull ? 0 : overlap.height)
            })
        keyboardObservers.append(
            center.addObserver(
                forName: UIResponder.keyboardWillHideNotification, object: nil, queue: .main
            ) { _ in push(0) })
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

        // Idempotent re-entry: setup is one-shot, but Rust may call us
        // again on session-restore. Resolve so the awaiting IPC future
        // doesn't hang.
        if interruptionObserver != nil {
            invoke.resolve([:])
            return
        }
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

        startPathMonitor()

        invoke.resolve([:])
    }

    /// Start NWPathMonitor and forward every interface change to Rust as a
    /// `networkPathChange` event. Rust's `ConnectionMonitor::handle_path_update`
    /// debounces and re-evaluates against the cached server connections — so
    /// a Wi-Fi → cellular transition flips us off the now-unreachable LAN
    /// URL and onto a remote / relay before mpv has a chance to hang on TCP.
    private func startPathMonitor() {
        guard pathMonitor == nil else { return }

        let monitor = NWPathMonitor()
        monitor.pathUpdateHandler = { [weak self] path in
            // NWPathMonitor's pathUpdateHandler is @Sendable in the iOS 17+
            // SDK, so we can't touch self directly here. Hop to main and
            // let the isolated method do the work — `path` is Sendable.
            DispatchQueue.main.async {
                self?.handlePathUpdate(path)
            }
        }
        monitor.start(queue: pathMonitorQueue)
        pathMonitor = monitor
    }

    /// Body of the NWPathMonitor handler. Runs on main; called from the
    /// monitor closure via `DispatchQueue.main.async` to keep the @Sendable
    /// closure free of non-Sendable captures (`self`, `[String: Any]`).
    private func handlePathUpdate(_ path: NWPath) {
        // Map the path's available interfaces to a stable name list so
        // Rust's `HashSet<String>` diff fires only on real transitions.
        let interfaceNames: [String] = path.availableInterfaces.map { $0.name }.sorted()

        let primaryType: String
        if path.usesInterfaceType(.wifi) {
            primaryType = "wifi"
        } else if path.usesInterfaceType(.cellular) {
            primaryType = "cellular"
        } else if path.usesInterfaceType(.wiredEthernet) {
            primaryType = "wired"
        } else if path.usesInterfaceType(.loopback) {
            primaryType = "loopback"
        } else if path.status == .satisfied {
            primaryType = "other"
        } else {
            primaryType = "none"
        }

        let payload: JSObject = [
            "interfaces": interfaceNames,
            "type": primaryType,
            "isExpensive": path.isExpensive,
            "isConstrained": path.isConstrained,
            "satisfied": path.status == .satisfied,
        ]

        lastPathSnapshotLock.lock()
        lastPathSnapshot = payload
        lastPathSnapshotLock.unlock()
        trigger("networkPathChange", data: payload)
    }

    @objc public func getNetworkInfo(_ invoke: Invoke) throws {
        lastPathSnapshotLock.lock()
        let snapshot: JSObject = lastPathSnapshot.isEmpty ? ["satisfied": false] : lastPathSnapshot
        lastPathSnapshotLock.unlock()
        invoke.resolve(snapshot)
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

    /// Forward `demuxer-cache-time` to Rust. Resolves with `-1.0` when the
    /// property is unavailable (no stream loaded, or demuxer hasn't filled
    /// yet); the Rust side translates that to `None`. We use a negative
    /// sentinel because `JSObject` won't accept nil values.
    @objc public func mpvGetDemuxerCacheTime(_ invoke: Invoke) throws {
        let value = mpv?.getDemuxerCacheTime() ?? -1.0
        invoke.resolve(["value": value])
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

    // MARK: - Recovery grace

    /// Toggle the background-task assertion around a playback recovery
    /// window. Begin is idempotent (one assertion at a time); the system
    /// expiration handler releases it if recovery outlives the grant
    /// (~30 s), after which the normal suspension rules apply again.
    @objc public func setRecoveryGrace(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(RecoveryGraceArgs.self)
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            if args.active {
                self.beginRecoveryGraceTask()
            } else {
                self.endRecoveryGraceTask()
            }
        }
        invoke.resolve([:])
    }

    private func beginRecoveryGraceTask() {
        guard recoveryGraceTask == .invalid else { return }
        recoveryGraceTask = UIApplication.shared.beginBackgroundTask(withName: "ramus-recovery") {
            [weak self] in
            DispatchQueue.main.async { self?.endRecoveryGraceTask() }
        }
    }

    private func endRecoveryGraceTask() {
        guard recoveryGraceTask != .invalid else { return }
        UIApplication.shared.endBackgroundTask(recoveryGraceTask)
        recoveryGraceTask = .invalid
    }

    // MARK: - Keyboard

    @objc public func dismissKeyboard(_ invoke: Invoke) throws {
        DispatchQueue.main.async { [weak self] in
            // The search bar sits beside the webview, not inside it, so
            // endEditing on the webview alone would never reach it.
            self?.searchBar?.resignFirstResponder()
            self?.webView?.endEditing(true)
        }
        invoke.resolve([:])
    }

    // MARK: - Native Search Bar

    @objc public func showNativeSearchBar(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(ShowSearchBarArgs.self)
        DispatchQueue.main.async { [weak self] in
            self?.presentSearchBar(
                initialText: args.initialQuery,
                top: args.top.map { CGFloat($0) },
                width: args.width.map { CGFloat($0) })
        }
        invoke.resolve([:])
    }

    @objc public func hideNativeSearchBar(_ invoke: Invoke) throws {
        DispatchQueue.main.async { [weak self] in
            self?.removeSearchBar()
        }
        invoke.resolve([:])
    }

    private func presentSearchBar(initialText: String, top: CGFloat?, width: CGFloat?) {
        guard let webView = webView, let parent = webView.superview else { return }
        if let strip = searchStrip {
            // Already up: the page remounted its search view (a rotation
            // across the two-pane breakpoint) and is re-reporting the slot.
            // Re-place the existing strip; its text and keyboard stay as
            // they are rather than being torn down and rebuilt.
            placeSearchStrip(strip, in: parent, top: top, width: width)
            return
        }

        let bar = UISearchBar()
        bar.delegate = self
        bar.text = initialText.isEmpty ? nil : initialText
        bar.placeholder = "Search"
        bar.searchBarStyle = .minimal
        bar.overrideUserInterfaceStyle = .dark
        bar.tintColor = .white
        // UIKit's own cancel button is unusable here: iPadOS never draws it
        // (documented), and on iPhone it is disabled the moment the bar
        // resigns first responder — which any tap into the page causes.
        // A separate button beside the bar stays tappable on both.
        bar.showsCancelButton = false
        bar.setContentHuggingPriority(.defaultLow, for: .horizontal)

        let strip = UIStackView(arrangedSubviews: [bar, makeSearchCancelButton()])
        strip.axis = .horizontal
        strip.alignment = .center
        strip.spacing = 4
        strip.isLayoutMarginsRelativeArrangement = true
        strip.directionalLayoutMargins = NSDirectionalEdgeInsets(
            top: 0, leading: 0, bottom: 0, trailing: 10)
        strip.overrideUserInterfaceStyle = .dark
        strip.translatesAutoresizingMaskIntoConstraints = false

        parent.addSubview(strip)
        placeSearchStrip(strip, in: parent, top: top, width: width)

        searchBar = bar
        searchStrip = strip
        bar.becomeFirstResponder()
    }

    /// The page reserves a slot for the bar at the top of its search view
    /// and reports where that slot is. On a phone it starts at the safe
    /// area; in the two-pane tablet layout the view sits below the
    /// navigation pane's toolbar and spans only that pane, so the bar
    /// must not cover the toolbar or the content pane's header.
    private func placeSearchStrip(_ strip: UIView, in parent: UIView, top: CGFloat?, width: CGFloat?) {
        NSLayoutConstraint.deactivate(searchStripPlacement)
        var constraints = [strip.leadingAnchor.constraint(equalTo: parent.leadingAnchor)]
        if let top, top > 0 {
            constraints.append(strip.topAnchor.constraint(equalTo: parent.topAnchor, constant: top))
        } else {
            constraints.append(
                strip.topAnchor.constraint(equalTo: parent.safeAreaLayoutGuide.topAnchor))
        }
        if let width, width > 0, width < parent.bounds.width - 1 {
            constraints.append(strip.widthAnchor.constraint(equalToConstant: width))
        } else {
            constraints.append(strip.trailingAnchor.constraint(equalTo: parent.trailingAnchor))
        }
        NSLayoutConstraint.activate(constraints)
        searchStripPlacement = constraints
    }

    private func makeSearchCancelButton() -> UIButton {
        let button: UIButton
        if #available(iOS 26.0, *) {
            // Matches the system search bar's own round glass close button.
            var config = UIButton.Configuration.glass()
            config.image = UIImage(
                systemName: "xmark",
                withConfiguration: UIImage.SymbolConfiguration(pointSize: 15, weight: .medium))
            config.cornerStyle = .capsule
            config.contentInsets = .zero
            button = UIButton(configuration: config)
            NSLayoutConstraint.activate([
                button.widthAnchor.constraint(equalToConstant: 40),
                button.heightAnchor.constraint(equalToConstant: 40),
            ])
        } else {
            button = UIButton(type: .system)
            button.setTitle("Cancel", for: .normal)
            button.titleLabel?.font = .systemFont(ofSize: 17)
            button.contentEdgeInsets = UIEdgeInsets(top: 8, left: 6, bottom: 8, right: 6)
        }
        button.tintColor = .white
        button.accessibilityLabel = "Cancel"
        button.setContentHuggingPriority(.required, for: .horizontal)
        button.setContentCompressionResistancePriority(.required, for: .horizontal)
        button.addTarget(self, action: #selector(searchCancelTapped), for: .touchUpInside)
        return button
    }

    @objc private func searchCancelTapped() {
        removeSearchBar()
        dispatchSearchEvent("nativeSearchCancel", detail: nil)
    }

    private func removeSearchBar() {
        searchBar?.resignFirstResponder()
        searchStrip?.removeFromSuperview()
        searchStripPlacement = []
        searchStrip = nil
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

class RecoveryGraceArgs: Decodable {
    let active: Bool
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
    /// Top of the page's search view in points from the window's top edge;
    /// nil or 0 anchors the bar at the safe area instead.
    let top: Double?
    /// Width of the page's search view in points; nil or 0 spans the window.
    let width: Double?
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
        // Dismiss the keyboard but keep the bar mounted — the search view
        // is still open (scrolling results, expanding a section), so the
        // user must be able to edit the query or hit Cancel. The bar is
        // only removed by Cancel or an explicit hideNativeSearchBar
        // (which fires when the search view unmounts after navigation).
        true
    }

    func searchBarSearchButtonClicked(_ searchBar: UISearchBar) {
        searchBar.resignFirstResponder()
    }
}

extension UIView {
    @objc func _ramus_nilAccessoryView() -> UIView? { nil }
}

@_cdecl("init_plugin_ramus_ios_bridge")
func initPlugin() -> Plugin {
    return MpvBridgePlugin()
}
