import Foundation
import Libmpv
import os

/// Thread-safe shutdown flag. Retained by pending MainActor Tasks so it remains
/// valid after MPVController and WakeupContext are deallocated.
private final class ShutdownFlag: @unchecked Sendable {
    // All access is synchronized through OSAllocatedUnfairLock.
    private let _value = OSAllocatedUnfairLock(initialState: false)
    var isSet: Bool { _value.withLock { $0 } }
    func set() { _value.withLock { $0 = true } }
}

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "MPVController")

/// Thin Swift wrapper around libmpv's C API for audio-only playback.
/// Inspired by IINA's MPVController — event loop on a background queue,
/// callbacks dispatched to @MainActor.
/// NOT @MainActor — mpv's wakeup callback runs on its core thread, and
/// Swift 6 inserts actor isolation assertions into @objc closure thunks
/// even for closures that don't touch @MainActor state. All public API
/// is called from AudioPlayer (@MainActor) which is fine since mpv's
/// client API is thread-safe. Callbacks dispatch to @MainActor via Task.
public final class MPVController: @unchecked Sendable {

    // MARK: - Callbacks

    /// Fires on every time-pos change (coalesced by mpv).
    var onPositionChange: ((Double) -> Void)?
    /// Fires when duration changes (new file loaded).
    var onDurationChange: ((Double) -> Void)?
    /// Fires when playlist-pos changes (track change, including natural advance).
    var onPlaylistPosChange: ((Int) -> Void)?
    /// Fires when pause state changes.
    var onPauseChange: ((Bool) -> Void)?
    /// Fires when paused-for-cache changes (buffering).
    var onBufferingChange: ((Bool) -> Void)?
    /// Fires when cache-buffering-state changes (0-100).
    var onCacheStateChange: ((Int) -> Void)?
    /// Fires when idle-active becomes true (player idle after playlist end).
    var onIdleActive: (() -> Void)?
    /// Fires when a file is loaded and ready (file-loaded event).
    var onFileLoaded: (() -> Void)?
    /// Fires when a file ends (end-file event). Passes the reason string.
    var onFileEnded: ((FileEndReason) -> Void)?

    // MARK: - Types

    public enum FileEndReason: Sendable {
        case eof
        case stop
        case quit
        case error(String)
        case redirect
        case unknown
    }

    // MARK: - Private State

    /// The mpv handle. Commands go through this (MainActor).
    private var mpv: OpaquePointer?
    /// Duplicate handle pointer for the event loop (nonisolated). Set once in init, never mutated.
    /// mpv's client API is thread-safe, so accessing from eventQueue is fine.
    nonisolated(unsafe) private var mpvForEvents: OpaquePointer?
    private let eventQueue = DispatchQueue(label: "com.raspsoft.ramus.mpv-events", qos: .userInteractive)
    /// Prevent the wakeup context from being deallocated while mpv holds a reference.
    private var wakeupContext: WakeupContext?

    // Property observer reply IDs — fileprivate so WakeupContext can access
    fileprivate enum ObserverID: UInt64 {
        case timePos = 1
        case duration = 2
        case pause = 3
        case playlistPos = 5
        case pausedForCache = 7
        case idleActive = 9
        case cacheBufferingState = 10
    }

    // MARK: - Init / Deinit

    public init() {
        guard let handle = mpv_create() else {
            log.error("mpv_create() failed")
            return
        }
        mpv = handle
        mpvForEvents = handle

        // Audio-only configuration
        setOption("vo", "null")                   // No video output
        setOption("vid", "no")                    // Don't process video tracks
        setOption("ao", "coreaudio")              // macOS audio output
        setOption("gapless-audio", "yes")         // True gapless playback
        setOption("prefetch-playlist", "yes")     // Pre-buffer next playlist entry
        setOption("audio-buffer", "0.5")          // 500ms audio buffer (200ms popped on GPU-heavy UI events)
        setOption("keep-open", "no")               // Advance to next playlist entry on EOF
        setOption("idle", "yes")                  // Stay alive when idle
        setOption("input-default-bindings", "no") // No keyboard bindings
        setOption("input-vo-keyboard", "no")      // No keyboard input
        setOption("terminal", "no")               // No terminal output
        setOption("msg-level", "all=warn")        // Reduce log noise
        setOption("load-scripts", "no")           // No scripts needed for audio-only playback

        let err = mpv_initialize(handle)
        guard err >= 0 else {
            log.error("mpv_initialize failed: \(String(cString: mpv_error_string(err)), privacy: .public)")
            mpv_destroy(handle)
            mpv = nil
            mpvForEvents = nil
            return
        }

        // Observe properties
        observeProperty("time-pos", format: MPV_FORMAT_DOUBLE, id: .timePos)
        observeProperty("duration", format: MPV_FORMAT_DOUBLE, id: .duration)
        observeProperty("pause", format: MPV_FORMAT_FLAG, id: .pause)
        observeProperty("playlist-pos", format: MPV_FORMAT_INT64, id: .playlistPos)
        observeProperty("paused-for-cache", format: MPV_FORMAT_FLAG, id: .pausedForCache)
        observeProperty("idle-active", format: MPV_FORMAT_FLAG, id: .idleActive)
        observeProperty("cache-buffering-state", format: MPV_FORMAT_INT64, id: .cacheBufferingState)

        // Set wakeup callback to process events.
        // The callback runs on mpv's core thread — must NEVER touch @MainActor state.
        // We store a raw pointer to self (bypassing Swift's type system) so that
        // no @MainActor isolation check is inserted by the compiler.
        // The raw pointer is only dereferenced inside Task { @MainActor in } blocks.
        let context = WakeupContext(
            queue: eventQueue,
            mpvHandle: handle,
            ownerPtr: Unmanaged.passUnretained(self).toOpaque()
        )
        wakeupContext = context
        let contextPtr = Unmanaged.passUnretained(context).toOpaque()
        mpv_set_wakeup_callback(handle, { ctx in
            guard let ctx else { return }
            let wctx = Unmanaged<WakeupContext>.fromOpaque(ctx).takeUnretainedValue()
            wctx.queue.async {
                wctx.drainEvents()
            }
        }, contextPtr)

        log.info("mpv initialized (gapless=yes, prefetch=yes, ao=coreaudio)")
    }

    deinit {
        // mpvForEvents is nonisolated(unsafe), safe to access in deinit.
        // It always points to the same handle as mpv (set once in init).
        if let handle = mpvForEvents {
            // Signal shutdown FIRST so any in-flight MainActor Tasks (dispatched
            // by drainEvents) no-op instead of dereferencing the freed owner pointer.
            wakeupContext?.shutdownFlag.set()
            // Clear the wakeup callback to prevent new drainEvents blocks
            // from being enqueued after we start tearing down.
            mpv_set_wakeup_callback(handle, nil, nil)
            // Drain any already-enqueued event blocks on the eventQueue.
            // Deadlock safety invariant: the sole owner (AudioPlayer) is @MainActor,
            // so this deinit always runs on MainActor. Pending MainActor Tasks from
            // drainEvents will see the shutdown flag and skip the pointer dereference.
            eventQueue.sync {}
            mpv_destroy(handle)
        }
    }

    // MARK: - Commands

    /// Load a file. Mode: "replace", "append", "append-play", "insert-next", "insert-at".
    func loadFile(_ url: String, mode: String = "replace") {
        command("loadfile", url, mode)
    }

    /// Load a file and insert at a specific playlist index.
    func loadFileAt(_ url: String, index: Int) {
        command("loadfile", url, "insert-at", "\(index)")
    }

    /// Play a specific playlist index.
    func playlistPlayIndex(_ index: Int) {
        setProperty("playlist-pos", int: index)
    }

    /// Remove a playlist entry at the given index.
    func playlistRemove(_ index: Int) {
        command("playlist-remove", "\(index)")
    }

    /// Move a playlist entry from one index to another.
    func playlistMove(from: Int, to: Int) {
        command("playlist-move", "\(from)", "\(to)")
    }

    /// Seek to an absolute position in seconds.
    func seek(to seconds: Double) {
        command("seek", "\(seconds)", "absolute")
    }

    /// Set pause state.
    func setPause(_ paused: Bool) {
        setProperty("pause", flag: paused)
    }

    /// Set volume (0-100+).
    func setVolume(_ volume: Double) {
        setProperty("volume", double: volume)
    }

    /// Set the audio filter chain (e.g. lavfi equalizer). Pass empty string to clear.
    func setAudioFilters(_ value: String) {
        setPropertyString("af", value)
    }

    /// Stop playback and clear the playlist.
    func stop() {
        command("stop")
    }

    /// Get current volume.
    func getVolume() -> Double {
        getPropertyDouble("volume")
    }

    // MARK: - Low-Level Property Access

    private func setOption(_ name: String, _ value: String) {
        guard let mpv else { return }
        let err = mpv_set_option_string(mpv, name, value)
        if err < 0 {
            log.warning("mpv set option \(name, privacy: .public)=\(value, privacy: .public) failed: \(String(cString: mpv_error_string(err)), privacy: .public)")
        }
    }

    private func setProperty(_ name: String, flag value: Bool) {
        guard let mpv else { return }
        var val: Int32 = value ? 1 : 0
        let err = mpv_set_property(mpv, name, MPV_FORMAT_FLAG, &val)
        if err < 0 {
            log.warning("mpv set \(name, privacy: .public)=\(value, privacy: .public) failed: \(String(cString: mpv_error_string(err)), privacy: .public)")
        }
    }

    private func setProperty(_ name: String, int value: Int) {
        guard let mpv else { return }
        var val = Int64(value)
        let err = mpv_set_property(mpv, name, MPV_FORMAT_INT64, &val)
        if err < 0 {
            log.warning("mpv set \(name, privacy: .public)=\(value, privacy: .public) failed: \(String(cString: mpv_error_string(err)), privacy: .public)")
        }
    }

    private func setProperty(_ name: String, double value: Double) {
        guard let mpv else { return }
        var val = value
        let err = mpv_set_property(mpv, name, MPV_FORMAT_DOUBLE, &val)
        if err < 0 {
            log.warning("mpv set \(name, privacy: .public)=\(value, privacy: .public) failed: \(String(cString: mpv_error_string(err)), privacy: .public)")
        }
    }

    private func getPropertyDouble(_ name: String) -> Double {
        guard let mpv else { return 0 }
        var value: Double = 0
        mpv_get_property(mpv, name, MPV_FORMAT_DOUBLE, &value)
        return value
    }

    private func setPropertyString(_ name: String, _ value: String) {
        guard let mpv else { return }
        let err = mpv_set_property_string(mpv, name, value)
        if err < 0 {
            log.warning("mpv set \(name, privacy: .public)=\(value, privacy: .public) failed: \(String(cString: mpv_error_string(err)), privacy: .public)")
        }
    }

    private func getPropertyInt(_ name: String) -> Int {
        guard let mpv else { return -1 }
        var value: Int64 = 0
        mpv_get_property(mpv, name, MPV_FORMAT_INT64, &value)
        return Int(value)
    }

    private func command(_ args: String...) {
        guard let mpv else { return }
        // mpv_command takes a null-terminated array of const char*
        let cStrings = args.map { strdup($0) }
        var cArgs = cStrings.map { UnsafePointer($0) as UnsafePointer<CChar>? }
        cArgs.append(nil)
        let err = mpv_command(mpv, &cArgs)
        cStrings.forEach { free($0) }
        if err < 0 {
            // Redact URLs to avoid leaking X-Plex-Token in os_log
            let redactedArgs = args.map { arg in
                arg.contains("X-Plex-Token") ? arg.replacingOccurrences(of: #"X-Plex-Token=[^&]*"#, with: "X-Plex-Token=REDACTED", options: .regularExpression) : arg
            }
            let cmdStr = redactedArgs.joined(separator: " ")
            log.warning("mpv command '\(cmdStr, privacy: .public)' failed: \(String(cString: mpv_error_string(err)), privacy: .public)")
        }
    }

    private func observeProperty(_ name: String, format: mpv_format, id: ObserverID) {
        guard let mpv else { return }
        mpv_observe_property(mpv, id.rawValue, name, format)
    }

}

/// Nonisolated context for mpv's wakeup callback.
/// Uses a raw pointer to the MPVController owner to completely bypass Swift's
/// actor isolation type checking. The raw pointer is ONLY dereferenced inside
/// `Task { @MainActor in }` blocks after checking the shutdown flag,
/// ensuring the owner is still alive.
private final class WakeupContext: @unchecked Sendable {
    let queue: DispatchQueue
    let mpvHandle: OpaquePointer
    /// Raw pointer to MPVController. NOT a strong/weak reference — just a pointer value.
    /// The MPVController must outlive this context (guaranteed by wakeupContext property).
    let ownerPtr: UnsafeMutableRawPointer
    /// Set by MPVController.deinit before teardown. Pending MainActor Tasks check this
    /// before dereferencing the owner pointer, preventing use-after-free.
    let shutdownFlag = ShutdownFlag()

    init(queue: DispatchQueue, mpvHandle: OpaquePointer, ownerPtr: UnsafeMutableRawPointer) {
        self.queue = queue
        self.mpvHandle = mpvHandle
        self.ownerPtr = ownerPtr
    }

    /// Drain all pending mpv events. Called on eventQueue (never main thread).
    /// Accesses mpvHandle directly (nonisolated C API, thread-safe).
    /// Routes callbacks to MainActor via Task {}.
    func drainEvents() {
        let mpv = mpvHandle
        let ptr = ownerPtr
        let bits = Int(bitPattern: ptr)  // Sendable version for Task closures
        let flag = shutdownFlag           // Retained by Tasks to outlive WakeupContext

        while true {
            let event = mpv_wait_event(mpv, 0)
            guard let event, event.pointee.event_id != MPV_EVENT_NONE else { break }

            switch event.pointee.event_id {
            case MPV_EVENT_PROPERTY_CHANGE:
                guard let prop = event.pointee.data?.assumingMemoryBound(to: mpv_event_property.self).pointee else { continue }
                let replyID = event.pointee.reply_userdata
                Self.handlePropertyChange(prop, replyID: replyID, ownerPtr: ptr, shutdownFlag: flag)

            case MPV_EVENT_FILE_LOADED:
                Task { @MainActor in
                    guard let owner = Self.safeOwner(from: bits, flag: flag) else { return }
                    owner.onFileLoaded?()
                }

            case MPV_EVENT_END_FILE:
                let reason: MPVController.FileEndReason
                if let endFile = event.pointee.data?.assumingMemoryBound(to: mpv_event_end_file.self).pointee {
                    switch endFile.reason {
                    case MPV_END_FILE_REASON_EOF: reason = .eof
                    case MPV_END_FILE_REASON_STOP: reason = .stop
                    case MPV_END_FILE_REASON_QUIT: reason = .quit
                    case MPV_END_FILE_REASON_ERROR:
                        let errStr = String(cString: mpv_error_string(endFile.error))
                        reason = .error(errStr)
                    case MPV_END_FILE_REASON_REDIRECT: reason = .redirect
                    default: reason = .unknown
                    }
                } else {
                    reason = .unknown
                }
                Task { @MainActor in
                    guard let owner = Self.safeOwner(from: bits, flag: flag) else { return }
                    owner.onFileEnded?(reason)
                }

            case MPV_EVENT_SHUTDOWN:
                log.info("mpv shutdown")
                return

            default:
                break
            }
        }
    }

    /// Resolve the owner pointer on MainActor, returning nil if shutdown has begun.
    @MainActor private static func safeOwner(from bits: Int, flag: ShutdownFlag) -> MPVController? {
        guard !flag.isSet else { return nil }
        return Unmanaged<MPVController>.fromOpaque(UnsafeMutableRawPointer(bitPattern: bits)!).takeUnretainedValue()
    }

    private static func handlePropertyChange(_ prop: mpv_event_property, replyID: UInt64, ownerPtr: UnsafeMutableRawPointer, shutdownFlag flag: ShutdownFlag) {
        guard let id = MPVController.ObserverID(rawValue: replyID) else { return }
        // Convert pointer to Int (Sendable) to cross the isolation boundary
        let bits = Int(bitPattern: ownerPtr)

        switch id {
        case .timePos:
            guard prop.format == MPV_FORMAT_DOUBLE, let data = prop.data else { return }
            let value = data.assumingMemoryBound(to: Double.self).pointee
            Task { @MainActor in safeOwner(from: bits, flag: flag)?.onPositionChange?(value) }

        case .duration:
            guard prop.format == MPV_FORMAT_DOUBLE, let data = prop.data else { return }
            let value = data.assumingMemoryBound(to: Double.self).pointee
            Task { @MainActor in safeOwner(from: bits, flag: flag)?.onDurationChange?(value) }

        case .pause:
            guard prop.format == MPV_FORMAT_FLAG, let data = prop.data else { return }
            let value = data.assumingMemoryBound(to: Int32.self).pointee != 0
            Task { @MainActor in safeOwner(from: bits, flag: flag)?.onPauseChange?(value) }

        case .playlistPos:
            guard prop.format == MPV_FORMAT_INT64, let data = prop.data else { return }
            let value = Int(data.assumingMemoryBound(to: Int64.self).pointee)
            Task { @MainActor in safeOwner(from: bits, flag: flag)?.onPlaylistPosChange?(value) }

        case .pausedForCache:
            guard prop.format == MPV_FORMAT_FLAG, let data = prop.data else { return }
            let value = data.assumingMemoryBound(to: Int32.self).pointee != 0
            Task { @MainActor in safeOwner(from: bits, flag: flag)?.onBufferingChange?(value) }

        case .idleActive:
            guard prop.format == MPV_FORMAT_FLAG, let data = prop.data else { return }
            let value = data.assumingMemoryBound(to: Int32.self).pointee != 0
            if value { Task { @MainActor in safeOwner(from: bits, flag: flag)?.onIdleActive?() } }

        case .cacheBufferingState:
            guard prop.format == MPV_FORMAT_INT64, let data = prop.data else { return }
            let value = Int(data.assumingMemoryBound(to: Int64.self).pointee)
            Task { @MainActor in safeOwner(from: bits, flag: flag)?.onCacheStateChange?(value) }
        }
    }
}
