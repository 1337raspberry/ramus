import Foundation
import MediaPlayer
import Models
#if os(macOS)
import AppKit
#endif

/// Bridges AudioPlayer state to MPRemoteCommandCenter (media keys) and
/// MPNowPlayingInfoCenter (Control Center / Now Playing widget).
@MainActor
public final class NowPlayingBridge {

    private let player: AudioPlayer
    private let artURLProvider: (String?) -> URL?
    private var cachedArtwork: MPMediaItemArtwork?
    private var cachedArtThumb: String?

    // nonisolated(unsafe): Swift 6 forbids accessing stored properties in deinit
    // of a @MainActor class. Set once in registerCommands(), cleaned up in deinit.
    // Safe because the sole owner (PlaybackViewModel) is @MainActor.
    nonisolated(unsafe) private var commandTargets: [Any] = []

    public init(player: AudioPlayer, artURLProvider: @escaping (String?) -> URL?) {
        self.player = player
        self.artURLProvider = artURLProvider
        registerCommands()
    }

    deinit {
        let center = MPRemoteCommandCenter.shared()
        for target in commandTargets {
            center.playCommand.removeTarget(target)
            center.pauseCommand.removeTarget(target)
            center.togglePlayPauseCommand.removeTarget(target)
            center.nextTrackCommand.removeTarget(target)
            center.previousTrackCommand.removeTarget(target)
            center.changePlaybackPositionCommand.removeTarget(target)
        }
    }

    // MARK: - Remote Commands

    private func registerCommands() {
        let center = MPRemoteCommandCenter.shared()

        commandTargets.append(center.playCommand.addTarget { [weak self] _ in
            Task { @MainActor in self?.player.resume() }
            return .success
        })
        commandTargets.append(center.pauseCommand.addTarget { [weak self] _ in
            Task { @MainActor in self?.player.pause() }
            return .success
        })
        commandTargets.append(center.togglePlayPauseCommand.addTarget { [weak self] _ in
            Task { @MainActor in self?.player.togglePlayPause() }
            return .success
        })
        commandTargets.append(center.nextTrackCommand.addTarget { [weak self] _ in
            Task { @MainActor in self?.player.next() }
            return .success
        })
        commandTargets.append(center.previousTrackCommand.addTarget { [weak self] _ in
            Task { @MainActor in self?.player.previous() }
            return .success
        })
        commandTargets.append(center.changePlaybackPositionCommand.addTarget { [weak self] event in
            guard let posEvent = event as? MPChangePlaybackPositionCommandEvent else {
                return .commandFailed
            }
            let time = posEvent.positionTime
            Task { @MainActor in self?.player.seek(to: time) }
            return .success
        })

        // Disable unused commands
        center.likeCommand.isEnabled = false
        center.dislikeCommand.isEnabled = false
        center.bookmarkCommand.isEnabled = false
    }

    // MARK: - Now Playing Info Updates

    /// Full update — call on track change.
    public func updateTrack() {
        let state = player.state
        guard let track = state.currentTrack else {
            clear()
            return
        }

        var info: [String: Any] = [
            MPMediaItemPropertyTitle: track.title,
            MPMediaItemPropertyArtist: track.displayArtist,
            MPMediaItemPropertyAlbumTitle: track.albumTitle,
            MPMediaItemPropertyPlaybackDuration: player.duration,
            MPNowPlayingInfoPropertyElapsedPlaybackTime: player.position,
            MPNowPlayingInfoPropertyPlaybackRate: state.status == .playing ? 1.0 : 0.0,
        ]

        if let index = track.index {
            info[MPMediaItemPropertyAlbumTrackNumber] = index
        }

        // Reuse cached artwork if same album thumb
        if let artwork = cachedArtwork, cachedArtThumb == track.thumb {
            info[MPMediaItemPropertyArtwork] = artwork
        }

        MPNowPlayingInfoCenter.default().nowPlayingInfo = info

        // Fetch artwork async if thumb changed
        if track.thumb != cachedArtThumb {
            fetchArtwork(thumb: track.thumb)
        }
    }

    /// Lightweight update — call on play/pause toggle.
    public func updatePlaybackState() {
        guard var info = MPNowPlayingInfoCenter.default().nowPlayingInfo else { return }
        let state = player.state
        info[MPNowPlayingInfoPropertyPlaybackRate] = state.status == .playing ? 1.0 : 0.0
        info[MPNowPlayingInfoPropertyElapsedPlaybackTime] = player.position
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }

    /// Clear now playing — call on stop/sign out.
    public func clear() {
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
        cachedArtwork = nil
        cachedArtThumb = nil
    }

    // MARK: - Artwork

    private func fetchArtwork(thumb: String?) {
        guard let url = artURLProvider(thumb) else { return }
        let thumbKey = thumb

        Task {
            do {
                let (data, _) = try await URLSession.shared.data(from: url)
                guard let artwork = Self.makeArtwork(from: data) else { return }

                // Only apply if still the same track
                guard self.player.state.currentTrack?.thumb == thumbKey else { return }
                self.cachedArtwork = artwork
                self.cachedArtThumb = thumbKey

                if var info = MPNowPlayingInfoCenter.default().nowPlayingInfo {
                    info[MPMediaItemPropertyArtwork] = artwork
                    MPNowPlayingInfoCenter.default().nowPlayingInfo = info
                }
            } catch {
                // Silent — now playing works fine without artwork
            }
        }
    }

    /// Build MPMediaItemArtwork off the main actor so the requestHandler
    /// closure doesn't capture @MainActor-isolated state.
    nonisolated private static func makeArtwork(from data: Data) -> MPMediaItemArtwork? {
        guard let image = NSImage(data: data) else { return nil }
        let size = image.size
        return MPMediaItemArtwork(boundsSize: size) { _ in image }
    }
}
