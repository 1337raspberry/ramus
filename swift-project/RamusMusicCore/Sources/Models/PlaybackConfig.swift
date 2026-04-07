import Foundation

/// User-configurable playback preferences. Constructed in the app target from
/// UserDefaults and passed into AudioPlayer via `configure`/`updateConfig`.
public struct PlaybackConfig: Sendable, Equatable {

    public enum PlaybackMode: String, Sendable {
        /// Always direct play. mpv handles all codecs natively.
        case directPlay
        /// Direct play locally. Transcode lossless when on a remote connection.
        case transcodeLosslessRemote
        /// Always transcode lossless formats to 256 kbps MP3 via HLS.
        case transcodeLossless
    }

    /// Playback mode — determines when to transcode vs direct play.
    public var playbackMode: PlaybackMode

    /// Number of upcoming tracks to prefetch (1–20).
    public var lookaheadDepth: Int

    /// Maximum audio cache size in bytes. Default 2 GB.
    public var audioCacheLimitBytes: Int64

    /// Default cache limit: 2 GB.
    public static let defaultCacheLimitBytes: Int64 = 2_147_483_648

    public static let `default` = PlaybackConfig(
        playbackMode: .directPlay,
        lookaheadDepth: 3,
        audioCacheLimitBytes: defaultCacheLimitBytes
    )

    public init(
        playbackMode: PlaybackMode,
        lookaheadDepth: Int = 3,
        audioCacheLimitBytes: Int64 = defaultCacheLimitBytes
    ) {
        self.playbackMode = playbackMode
        self.lookaheadDepth = max(1, min(20, lookaheadDepth))
        self.audioCacheLimitBytes = audioCacheLimitBytes
    }
}
