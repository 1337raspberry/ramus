import Foundation
import MediaPlayer
import Tauri
import UIKit

/// `MPNowPlayingInfoCenter` + `MPRemoteCommandCenter` wrapper. The bridge
/// emits `remote…` events for every user action on the lock-screen / CC
/// widget, and its `update(_:)` method pushes metadata + state back
/// whenever Rust's playback state changes.
///
/// Artwork loading is async and best-effort: if the `coverUrl` is
/// `file://` or `https://`, we fetch it off the main actor and write
/// back via `MPMediaItemArtwork`. A failed fetch simply leaves the
/// previous artwork in place.
final class NowPlayingBridge {
    /// Called with the event name and JSON payload whenever a remote
    /// command fires. `JSObject` is Tauri's `[String: JSValue]` alias —
    /// the only form `Plugin.trigger(_:data:)` accepts. Empty dict
    /// literals and `["key": Double]` literals bridge automatically.
    typealias Emit = (_ name: String, _ data: JSObject) -> Void

    private let emit: Emit
    private var lastArtworkUrl: String?

    init(emit: @escaping Emit) {
        self.emit = emit
        configureRemoteCommands()
    }

    // MARK: - Public API

    func update(_ meta: NowPlayingMetadata) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            var info: [String: Any] = [
                MPMediaItemPropertyTitle: meta.title,
                MPMediaItemPropertyArtist: meta.artist,
                MPMediaItemPropertyAlbumTitle: meta.album,
                MPMediaItemPropertyPlaybackDuration: meta.duration,
                MPNowPlayingInfoPropertyElapsedPlaybackTime: meta.position,
                MPNowPlayingInfoPropertyPlaybackRate: meta.isPlaying ? 1.0 : 0.0,
            ]

            // Preserve existing artwork when only transport state changes;
            // otherwise the lock-screen flickers the cover to black each tick.
            let infoCenter = MPNowPlayingInfoCenter.default()
            if let existing = infoCenter.nowPlayingInfo?[MPMediaItemPropertyArtwork] {
                info[MPMediaItemPropertyArtwork] = existing
            }

            infoCenter.nowPlayingInfo = info
            infoCenter.playbackState = meta.isPlaying ? .playing : .paused

            if let coverUrl = meta.coverUrl, coverUrl != self.lastArtworkUrl {
                self.lastArtworkUrl = coverUrl
                self.loadArtwork(from: coverUrl)
            }
        }
    }

    func clear() {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            let infoCenter = MPNowPlayingInfoCenter.default()
            infoCenter.nowPlayingInfo = nil
            infoCenter.playbackState = .stopped
            self.lastArtworkUrl = nil
        }
    }

    // MARK: - Remote command wiring

    private func configureRemoteCommands() {
        let center = MPRemoteCommandCenter.shared()

        center.playCommand.addTarget { [weak self] _ in
            self?.emit("remotePlay", [:])
            return .success
        }
        center.pauseCommand.addTarget { [weak self] _ in
            self?.emit("remotePause", [:])
            return .success
        }
        center.togglePlayPauseCommand.addTarget { [weak self] _ in
            self?.emit("remoteToggle", [:])
            return .success
        }
        center.nextTrackCommand.addTarget { [weak self] _ in
            self?.emit("remoteNext", [:])
            return .success
        }
        center.previousTrackCommand.addTarget { [weak self] _ in
            self?.emit("remotePrevious", [:])
            return .success
        }
        center.changePlaybackPositionCommand.addTarget { [weak self] event in
            guard let positionEvent = event as? MPChangePlaybackPositionCommandEvent else {
                return .commandFailed
            }
            self?.emit("remoteSeek", ["position": positionEvent.positionTime])
            return .success
        }

        // Not used, but the docs are explicit: commands default to
        // enabled, so anything we don't handle should be turned off
        // to avoid a disabled-looking UI.
        [center.likeCommand, center.dislikeCommand, center.bookmarkCommand].forEach { $0.isEnabled = false }
    }

    // MARK: - Artwork

    private func loadArtwork(from urlString: String) {
        guard let url = URL(string: urlString) else { return }

        if url.isFileURL {
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                guard let image = UIImage(contentsOfFile: url.path) else { return }
                DispatchQueue.main.async {
                    self?.setArtwork(image)
                }
            }
            return
        }

        // Copying `self` into the Task pre-hop avoids the Swift 6
        // Sendable warning about capturing the outer `self` across the
        // `MainActor.run` boundary.
        let bridge = self
        Task.detached {
            guard let (data, _) = try? await URLSession.shared.data(from: url),
                  let image = UIImage(data: data) else { return }
            await MainActor.run {
                bridge.setArtwork(image)
            }
        }
    }

    private func setArtwork(_ image: UIImage) {
        let art = MPMediaItemArtwork(boundsSize: image.size) { _ in image }
        var info = MPNowPlayingInfoCenter.default().nowPlayingInfo ?? [:]
        info[MPMediaItemPropertyArtwork] = art
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }
}
