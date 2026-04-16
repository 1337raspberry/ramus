import Foundation
import Libmpv
import os

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "MpvController")

/// Thin Swift wrapper around libmpv's C API for iOS audio-only playback.
/// Ported from the reference Swift app at
/// /Users/sam/projects/ramus-ios/RamusMusicCore/Sources/Playback/MPVController.swift
/// — trimmed of buffering/cache observers we don't surface yet, and
/// de-`@MainActor`-ified because Tauri plugin callbacks already run off
/// the UI thread.
///
/// The event-loop context (`WakeupContext`) is retained by the controller
/// so it stays alive while libmpv holds a pointer to it. Drop order
/// matters: `deinit` first clears the wakeup callback, then destroys
/// the mpv handle.
final class MpvController {
    // MARK: - Callbacks (set by the plugin, invoked on background queue)

    var onPositionChange: ((Double) -> Void)?
    var onDurationChange: ((Double) -> Void)?
    var onPlaylistPosChange: ((Int) -> Void)?
    var onPauseChange: ((Bool) -> Void)?
    var onIdleActive: (() -> Void)?
    var onFileLoaded: (() -> Void)?
    var onFileEnded: ((String) -> Void)?

    // MARK: - State

    private var mpv: OpaquePointer?
    private let eventQueue = DispatchQueue(
        label: "com.raspsoft.ramus.mpv-events",
        qos: .userInteractive
    )
    private var wakeupContext: WakeupContext?

    fileprivate enum ObserverID: UInt64 {
        case timePos = 1
        case duration = 2
        case pause = 3
        case playlistPos = 5
        case idleActive = 9
    }

    // MARK: - Lifecycle

    init() {
        guard let handle = mpv_create() else {
            log.error("mpv_create() failed")
            return
        }
        mpv = handle

        // Audio-only options. Matches the reference Swift app's
        // configuration; `audio-exclusive=yes` is required so the lock
        // screen can drive playback state cleanly.
        setOption("vo", "null")
        setOption("vid", "no")
        setOption("ao", "audiounit")
        setOption("audio-exclusive", "yes")
        setOption("gapless-audio", "yes")
        setOption("prefetch-playlist", "yes")
        setOption("audio-buffer", "0.5")
        setOption("keep-open", "no")
        setOption("idle", "yes")
        setOption("input-default-bindings", "no")
        setOption("input-vo-keyboard", "no")
        setOption("terminal", "no")
        setOption("msg-level", "all=warn")
        setOption("load-scripts", "no")

        let err = mpv_initialize(handle)
        guard err >= 0 else {
            let msg = String(cString: mpv_error_string(err))
            log.error("mpv_initialize failed: \(msg, privacy: .public)")
            mpv_destroy(handle)
            mpv = nil
            return
        }

        observeProperty("time-pos", format: MPV_FORMAT_DOUBLE, id: .timePos)
        observeProperty("duration", format: MPV_FORMAT_DOUBLE, id: .duration)
        observeProperty("pause", format: MPV_FORMAT_FLAG, id: .pause)
        observeProperty("playlist-pos", format: MPV_FORMAT_INT64, id: .playlistPos)
        observeProperty("idle-active", format: MPV_FORMAT_FLAG, id: .idleActive)

        let context = WakeupContext(
            queue: eventQueue,
            mpvHandle: handle,
            owner: self
        )
        wakeupContext = context
        let ctxPtr = Unmanaged.passUnretained(context).toOpaque()
        mpv_set_wakeup_callback(handle, { opaque in
            guard let opaque else { return }
            let wctx = Unmanaged<WakeupContext>.fromOpaque(opaque).takeUnretainedValue()
            wctx.queue.async { wctx.drainEvents() }
        }, ctxPtr)

        log.info("mpv initialized (ao=audiounit, gapless=yes)")
    }

    deinit {
        if let handle = mpv {
            wakeupContext?.shutdown = true
            mpv_set_wakeup_callback(handle, nil, nil)
            eventQueue.sync {} // flush any in-flight drains
            mpv_destroy(handle)
        }
    }

    // MARK: - Commands

    func loadFile(_ url: String, mode: String, options: String?) {
        if let options, !options.isEmpty {
            command("loadfile", url, mode, options)
        } else {
            command("loadfile", url, mode)
        }
    }

    func loadFileAt(_ url: String, index: Int, options: String?) {
        if let options, !options.isEmpty {
            command("loadfile", url, "insert-at", "\(index)", options)
        } else {
            command("loadfile", url, "insert-at", "\(index)")
        }
    }

    func playlistPlayIndex(_ index: Int) {
        setProperty("playlist-pos", int: index)
    }

    func playlistRemove(_ index: Int) {
        command("playlist-remove", "\(index)")
    }

    func playlistMove(from: Int, to: Int) {
        command("playlist-move", "\(from)", "\(to)")
    }

    func seek(to seconds: Double) {
        command("seek", "\(seconds)", "absolute")
    }

    func setPause(_ paused: Bool) {
        setProperty("pause", flag: paused)
    }

    func setVolume(_ volume: Double) {
        setProperty("volume", double: volume)
    }

    func getVolume() -> Double {
        guard let mpv else { return 0 }
        var value: Double = 0
        mpv_get_property(mpv, "volume", MPV_FORMAT_DOUBLE, &value)
        return value
    }

    func setAudioFilters(_ value: String) {
        setPropertyString("af", value)
    }

    func stop() {
        command("stop")
    }

    // MARK: - Low-level helpers

    private func setOption(_ name: String, _ value: String) {
        guard let mpv else { return }
        let err = mpv_set_option_string(mpv, name, value)
        if err < 0 {
            let msg = String(cString: mpv_error_string(err))
            log.warning("mpv set option \(name, privacy: .public) failed: \(msg, privacy: .public)")
        }
    }

    private func setProperty(_ name: String, flag value: Bool) {
        guard let mpv else { return }
        var val: Int32 = value ? 1 : 0
        let err = mpv_set_property(mpv, name, MPV_FORMAT_FLAG, &val)
        if err < 0 {
            let msg = String(cString: mpv_error_string(err))
            log.warning("mpv set \(name, privacy: .public) failed: \(msg, privacy: .public)")
        }
    }

    private func setProperty(_ name: String, int value: Int) {
        guard let mpv else { return }
        var val = Int64(value)
        let err = mpv_set_property(mpv, name, MPV_FORMAT_INT64, &val)
        if err < 0 {
            let msg = String(cString: mpv_error_string(err))
            log.warning("mpv set \(name, privacy: .public) failed: \(msg, privacy: .public)")
        }
    }

    private func setProperty(_ name: String, double value: Double) {
        guard let mpv else { return }
        var val = value
        let err = mpv_set_property(mpv, name, MPV_FORMAT_DOUBLE, &val)
        if err < 0 {
            let msg = String(cString: mpv_error_string(err))
            log.warning("mpv set \(name, privacy: .public) failed: \(msg, privacy: .public)")
        }
    }

    private func setPropertyString(_ name: String, _ value: String) {
        guard let mpv else { return }
        let err = mpv_set_property_string(mpv, name, value)
        if err < 0 {
            let msg = String(cString: mpv_error_string(err))
            log.warning("mpv set \(name, privacy: .public) failed: \(msg, privacy: .public)")
        }
    }

    private func command(_ args: String...) {
        guard let mpv else { return }
        let cStrings = args.map { strdup($0) }
        var cArgs = cStrings.map { UnsafePointer($0) as UnsafePointer<CChar>? }
        cArgs.append(nil)
        let err = mpv_command(mpv, &cArgs)
        cStrings.forEach { free($0) }
        if err < 0 {
            // Redact tokens before logging.
            let safe = args.map { arg in
                arg.replacingOccurrences(
                    of: #"X-Plex-Token=[^&]*"#,
                    with: "X-Plex-Token=REDACTED",
                    options: .regularExpression
                )
            }
            let msg = String(cString: mpv_error_string(err))
            log.warning("mpv cmd '\(safe.joined(separator: " "), privacy: .public)' failed: \(msg, privacy: .public)")
        }
    }

    private func observeProperty(_ name: String, format: mpv_format, id: ObserverID) {
        guard let mpv else { return }
        mpv_observe_property(mpv, id.rawValue, name, format)
    }
}

/// Event-loop context owned by `MpvController` and referenced by libmpv's
/// wakeup callback via an unretained opaque pointer. Controller's deinit
/// sets `shutdown = true` then clears the wakeup callback, so no drain
/// can dereference a freed owner.
private final class WakeupContext {
    let queue: DispatchQueue
    let mpvHandle: OpaquePointer
    weak var owner: MpvController?
    var shutdown = false

    init(queue: DispatchQueue, mpvHandle: OpaquePointer, owner: MpvController) {
        self.queue = queue
        self.mpvHandle = mpvHandle
        self.owner = owner
    }

    func drainEvents() {
        while !shutdown {
            guard let event = mpv_wait_event(mpvHandle, 0) else { break }
            if event.pointee.event_id == MPV_EVENT_NONE { break }

            switch event.pointee.event_id {
            case MPV_EVENT_PROPERTY_CHANGE:
                guard let rawProp = event.pointee.data else { continue }
                let prop = rawProp.assumingMemoryBound(to: mpv_event_property.self).pointee
                handlePropertyChange(prop, replyID: event.pointee.reply_userdata)

            case MPV_EVENT_FILE_LOADED:
                owner?.onFileLoaded?()

            case MPV_EVENT_END_FILE:
                let reason: String
                if let data = event.pointee.data {
                    let endFile = data.assumingMemoryBound(to: mpv_event_end_file.self).pointee
                    switch endFile.reason {
                    case MPV_END_FILE_REASON_EOF: reason = "eof"
                    case MPV_END_FILE_REASON_STOP: reason = "stop"
                    case MPV_END_FILE_REASON_QUIT: reason = "quit"
                    case MPV_END_FILE_REASON_ERROR: reason = "error"
                    case MPV_END_FILE_REASON_REDIRECT: reason = "redirect"
                    default: reason = "unknown"
                    }
                } else {
                    reason = "unknown"
                }
                owner?.onFileEnded?(reason)

            case MPV_EVENT_SHUTDOWN:
                log.info("mpv shutdown")
                return

            default:
                break
            }
        }
    }

    private func handlePropertyChange(_ prop: mpv_event_property, replyID: UInt64) {
        guard let id = MpvController.ObserverID(rawValue: replyID) else { return }

        switch id {
        case .timePos:
            guard prop.format == MPV_FORMAT_DOUBLE, let data = prop.data else { return }
            owner?.onPositionChange?(data.assumingMemoryBound(to: Double.self).pointee)

        case .duration:
            guard prop.format == MPV_FORMAT_DOUBLE, let data = prop.data else { return }
            owner?.onDurationChange?(data.assumingMemoryBound(to: Double.self).pointee)

        case .pause:
            guard prop.format == MPV_FORMAT_FLAG, let data = prop.data else { return }
            owner?.onPauseChange?(data.assumingMemoryBound(to: Int32.self).pointee != 0)

        case .playlistPos:
            guard prop.format == MPV_FORMAT_INT64, let data = prop.data else { return }
            owner?.onPlaylistPosChange?(Int(data.assumingMemoryBound(to: Int64.self).pointee))

        case .idleActive:
            guard prop.format == MPV_FORMAT_FLAG, let data = prop.data else { return }
            if data.assumingMemoryBound(to: Int32.self).pointee != 0 {
                owner?.onIdleActive?()
            }
        }
    }
}
