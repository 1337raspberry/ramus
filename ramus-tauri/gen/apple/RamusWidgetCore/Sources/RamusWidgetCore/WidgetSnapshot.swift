import CoreGraphics
import Foundation

/// The playback snapshot the app writes for the widget. Mirrors
/// `ramus_core::widget::WidgetSnapshot`; times are seconds since 1970.
public struct WidgetSnapshot: Decodable, Equatable {
    public static let schema = 1

    public enum State: String, Decodable, Equatable {
        case playing, paused, idle, signedOut
    }

    public struct Track: Decodable, Equatable {
        public let ratingKey: String
        public let title: String
        public let artist: String
        public let album: String
        public let durationSec: Double
    }

    public struct Resume: Decodable, Equatable {
        public let title: String
        public let artist: String
    }

    public struct Colors: Decodable, Equatable {
        public let topLeft: String
        public let topRight: String
        public let bottomRight: String
        public let bottomLeft: String
        public let accent: String
        /// Where the corners came from: `art` (computed from this cover),
        /// `server`, `palette` or `default` (the brand gradient).
        public let source: String?

        public init(
            topLeft: String, topRight: String, bottomRight: String, bottomLeft: String,
            accent: String, source: String?
        ) {
            self.topLeft = topLeft
            self.topRight = topRight
            self.bottomRight = bottomRight
            self.bottomLeft = bottomLeft
            self.accent = accent
            self.source = source
        }

        /// The colours as painted: stand-in corners are replaced by ones
        /// computed from `cover`, and album corners get the app's tone pass.
        /// The brand gradient is already in its display form.
        public func resolved(cover: CGImage?) -> Colors {
            var corners = CornerHex(topLeft: topLeft, topRight: topRight, bottomRight: bottomRight, bottomLeft: bottomLeft)
            var source = self.source
            if source != "art", let cover, let computed = CornerColors.fromImage(cover) {
                corners = computed
                source = "art"
            }
            guard source != "default" else { return self }
            return Colors(
                topLeft: BackdropTone.display(corners.topLeft),
                topRight: BackdropTone.display(corners.topRight),
                bottomRight: BackdropTone.display(corners.bottomRight),
                bottomLeft: BackdropTone.display(corners.bottomLeft),
                accent: accent,
                source: source
            )
        }
    }

    public let schema: Int
    public let revision: Int
    public let state: State
    public let track: Track?
    public let positionSec: Double
    public let positionAt: Double
    public let lastActivityAt: Double
    public let resumable: Bool
    public let resume: Resume?
    public let colors: Colors?
    public let waveform: [Double]?
    public let cast: String?

    public static func decode(_ data: Data) throws -> WidgetSnapshot {
        try JSONDecoder().decode(WidgetSnapshot.self, from: data)
    }

    /// The snapshot in `dir`, or nil when it is missing, unreadable, or
    /// written in a layout this build doesn't know.
    public static func load(from dir: URL) -> WidgetSnapshot? {
        guard let data = try? Data(contentsOf: dir.appendingPathComponent(SharedContainer.snapshotFile)),
              let snapshot = try? decode(data),
              snapshot.schema == schema
        else { return nil }
        return snapshot
    }
}
