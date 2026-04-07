import Foundation
import Models
import PlexAPI
#if os(macOS)
import AppKit
#endif

/// Reports playback timeline updates to the Plex server so activity
/// appears on the dashboard ("Now Playing") and tracks get scrobbled.
///
/// Owns the session lifecycle: tracks the "active" session and always
/// sends `state=stopped` for the previous track/session before reporting
/// a new one as `playing`. This prevents ghost sessions stacking up
/// on the Plex dashboard when skipping tracks or switching albums.
@MainActor
public final class SessionReporter {

    private let player: AudioPlayer
    private let client: PlexClient

    /// The session ID currently being reported to Plex.
    private var activeSessionID: String?
    /// The track currently being reported as playing/paused.
    private var activeTrack: Track?
    /// Position of the active track at last report (for stop reports).
    private var activePosition: TimeInterval = 0
    /// Duration of the active track (for stop reports).
    private var activeDuration: TimeInterval = 0

    // nonisolated(unsafe): Swift 6 forbids accessing stored properties in deinit
    // of a @MainActor class. These are set once (init/setup) and cleaned up in
    // deinit. Safe because the sole owner (PlaybackViewModel) is @MainActor,
    // guaranteeing main-thread deallocation.
    nonisolated(unsafe) private var reportTimer: Timer?
    private var scrobbledTrackKey: String?
    nonisolated(unsafe) private var terminationObserver: NSObjectProtocol?

    private static let reportInterval: TimeInterval = 10

    public init(player: AudioPlayer, client: PlexClient) {
        self.player = player
        self.client = client
        observeAppTermination()
    }

    deinit {
        reportTimer?.invalidate()
        if let o = terminationObserver {
            NotificationCenter.default.removeObserver(o)
        }
    }

    // MARK: - Event Hooks

    /// Call when a new track starts playing (from onTrackChange).
    /// Updates the active track info and sends `state=playing` on the current session.
    /// Does NOT send `state=stopped` for the previous track — within a queue, the
    /// same session ID persists and Plex transitions the transcode automatically.
    /// Session stops are handled by `playbackStopped()` (called on queue change/stop).
    public func trackStarted() {
        stopPeriodicReporting()
        activeSessionID = player.playSessionID
        activeTrack = player.state.currentTrack
        activePosition = 0
        activeDuration = player.duration
        scrobbledTrackKey = nil
        sendTimeline(state: "playing")
        startPeriodicReporting()
    }

    /// Call when a track finishes naturally (auto-advance). Scrobbles the ended track.
    /// Does NOT send state=stopped or clear active state — trackStarted() captures
    /// the old session info and sends a delayed stop after the new transcode is established.
    public func trackEnded(_ track: Track) {
        let ratingKey = track.ratingKey
        Task {
            await client.scrobble(ratingKey: ratingKey)
        }
    }

    /// Call when playback is paused.
    public func playbackPaused() {
        stopPeriodicReporting()
        snapshotPosition()
        sendTimeline(state: "paused")
    }

    /// Call when playback resumes from pause.
    public func playbackResumed() {
        snapshotPosition()
        sendTimeline(state: "playing")
        startPeriodicReporting()
    }

    /// Call when playback stops (end of queue or user stop).
    public func playbackStopped() {
        stopActiveSession()
    }

    /// Call when user seeks — send an immediate position update.
    public func playbackSeeked() {
        snapshotPosition()
        let state = player.state.status == .playing ? "playing" : "paused"
        sendTimeline(state: state)
    }

    // MARK: - Active Session Management

    /// Send `state=stopped` for whatever is currently active, then clear state.
    private func stopActiveSession() {
        stopPeriodicReporting()
        snapshotPosition()

        guard let track = activeTrack, let session = activeSessionID else { return }

        let ratingKey = track.ratingKey
        let timeMs = Int(activePosition * 1000)
        let durationMs = Int(activeDuration * 1000)

        Task {
            await client.reportTimeline(
                ratingKey: ratingKey,
                state: "stopped",
                timeMs: timeMs,
                durationMs: durationMs,
                sessionIdentifier: session
            )
        }

        activeTrack = nil
        activeSessionID = nil
    }

    /// Send a synchronous (blocking) stop for app termination.
    /// Uses a semaphore because NSApplication.willTerminate doesn't wait for async.
    private func stopActiveSessionSync() {
        guard let track = activeTrack, let session = activeSessionID,
              let serverURL = client.serverURL, let token = client.token else { return }

        let ratingKey = track.ratingKey
        let timeMs = Int(activePosition * 1000)
        let durationMs = Int(activeDuration * 1000)
        let clientId = client.clientIdentifier

        // Build the request inline — can't call async PlexClient from termination handler
        var components = URLComponents(
            url: serverURL.appendingPathComponent("/:/timeline"),
            resolvingAgainstBaseURL: false
        )!
        components.queryItems = [
            URLQueryItem(name: "ratingKey", value: ratingKey),
            URLQueryItem(name: "key", value: "/library/metadata/\(ratingKey)"),
            URLQueryItem(name: "state", value: "stopped"),
            URLQueryItem(name: "time", value: "\(timeMs)"),
            URLQueryItem(name: "duration", value: "\(durationMs)"),
            URLQueryItem(name: "identifier", value: "com.plexapp.plugins.library"),
            URLQueryItem(name: "X-Plex-Token", value: token),
        ]

        var request = URLRequest(url: components.url!)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue(clientId, forHTTPHeaderField: "X-Plex-Client-Identifier")
        request.setValue("ramus", forHTTPHeaderField: "X-Plex-Product")
        request.setValue("macOS", forHTTPHeaderField: "X-Plex-Platform")
        request.setValue("Mac", forHTTPHeaderField: "X-Plex-Device")
        // Note: headers built inline here because PlexClient.applyStandardHeaders
        // is not accessible from this synchronous termination handler.
        request.setValue(session, forHTTPHeaderField: "X-Plex-Session-Identifier")

        let semaphore = DispatchSemaphore(value: 0)
        let task = URLSession.shared.dataTask(with: request) { _, _, _ in
            semaphore.signal()
        }
        task.resume()
        _ = semaphore.wait(timeout: .now() + 2) // 2s max wait on quit
    }

    // MARK: - App Termination

    private func observeAppTermination() {
        #if os(macOS)
        terminationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification,
            object: nil, queue: .main
        ) { [weak self] _ in
            // Must run synchronously — app is about to die
            MainActor.assumeIsolated {
                self?.stopActiveSessionSync()
            }
        }
        #endif
    }

    // MARK: - Periodic Reporting

    private func startPeriodicReporting() {
        stopPeriodicReporting()
        reportTimer = Timer.scheduledTimer(withTimeInterval: Self.reportInterval, repeats: true) { [weak self] _ in
            Task { @MainActor in
                self?.periodicTick()
            }
        }
    }

    private func stopPeriodicReporting() {
        reportTimer?.invalidate()
        reportTimer = nil
    }

    private func periodicTick() {
        guard player.state.status == .playing,
              player.state.currentTrack != nil else {
            stopPeriodicReporting()
            return
        }

        snapshotPosition()
        sendTimeline(state: "playing")
        checkScrobble()
    }

    // MARK: - Scrobble

    private func checkScrobble() {
        guard let track = player.state.currentTrack,
              player.duration > 0,
              scrobbledTrackKey != track.ratingKey else { return }

        let progress = player.position / player.duration
        if progress >= 0.9 {
            scrobbledTrackKey = track.ratingKey
            let ratingKey = track.ratingKey
            Task {
                await client.scrobble(ratingKey: ratingKey)
            }
        }
    }

    // MARK: - Helpers

    /// Capture current position/duration so stop reports use accurate values.
    private func snapshotPosition() {
        activePosition = player.position
        activeDuration = player.duration
    }

    private func sendTimeline(state: String) {
        guard let track = activeTrack, let session = activeSessionID else { return }

        let timeMs = Int(player.position * 1000)
        let durationMs = Int(player.duration * 1000)
        let ratingKey = track.ratingKey

        Task {
            await client.reportTimeline(
                ratingKey: ratingKey,
                state: state,
                timeMs: timeMs,
                durationMs: durationMs,
                sessionIdentifier: session
            )
        }
    }
}
