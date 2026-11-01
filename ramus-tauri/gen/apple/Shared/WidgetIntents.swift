import AppIntents
import Foundation

// Buttons on the home-screen widget. Each conforms to AudioPlaybackIntent,
// so iOS runs `perform()` in the app's process (launching it in the
// background if it isn't running) rather than in the widget extension.
// The file is compiled into both targets: the widget needs the types to
// build its buttons, and only the app (RAMUS_APP) carries the call into
// Rust.

enum WidgetIntentRunner {
    /// What the widget was showing when a button was pressed.
    static func expect(_ ratingKey: String?, _ revision: Int) -> [String: Any] {
        var e: [String: Any] = ["revision": revision]
        if let ratingKey { e["ratingKey"] = ratingKey }
        return e
    }

    @discardableResult
    static func send(_ command: [String: Any]) async -> Int32 {
        #if RAMUS_APP
        guard let data = try? JSONSerialization.data(withJSONObject: command),
              let json = String(data: data, encoding: .utf8)
        else { return 2 }
        return await WidgetCommandBridge.send(json)
        #else
        return 0
        #endif
    }
}

struct PlayIntent: AudioPlaybackIntent {
    static var title: LocalizedStringResource = "Play or Pause"
    static var isDiscoverable = false

    @Parameter(title: "Play") var play: Bool
    @Parameter(title: "Track") var ratingKey: String?
    @Parameter(title: "Revision") var revision: Int

    init() {}
    init(play: Bool, ratingKey: String?, revision: Int) {
        self.play = play
        self.ratingKey = ratingKey
        self.revision = revision
    }

    func perform() async throws -> some IntentResult {
        await WidgetIntentRunner.send([
            "action": "play", "play": play, "expect": WidgetIntentRunner.expect(ratingKey, revision),
        ])
        return .result()
    }
}

struct NextIntent: AudioPlaybackIntent {
    static var title: LocalizedStringResource = "Next Track"
    static var isDiscoverable = false

    @Parameter(title: "Track") var ratingKey: String?
    @Parameter(title: "Revision") var revision: Int

    init() {}
    init(ratingKey: String?, revision: Int) {
        self.ratingKey = ratingKey
        self.revision = revision
    }

    func perform() async throws -> some IntentResult {
        await WidgetIntentRunner.send(["action": "next", "expect": WidgetIntentRunner.expect(ratingKey, revision)])
        return .result()
    }
}

struct PreviousIntent: AudioPlaybackIntent {
    static var title: LocalizedStringResource = "Previous Track"
    static var isDiscoverable = false

    @Parameter(title: "Track") var ratingKey: String?
    @Parameter(title: "Revision") var revision: Int

    init() {}
    init(ratingKey: String?, revision: Int) {
        self.ratingKey = ratingKey
        self.revision = revision
    }

    func perform() async throws -> some IntentResult {
        await WidgetIntentRunner.send(["action": "previous", "expect": WidgetIntentRunner.expect(ratingKey, revision)])
        return .result()
    }
}

struct SeekIntent: AudioPlaybackIntent {
    static var title: LocalizedStringResource = "Seek"
    static var isDiscoverable = false

    @Parameter(title: "Fraction") var fraction: Double
    @Parameter(title: "Track") var ratingKey: String?
    @Parameter(title: "Revision") var revision: Int

    init() {}
    init(fraction: Double, ratingKey: String?, revision: Int) {
        self.fraction = fraction
        self.ratingKey = ratingKey
        self.revision = revision
    }

    func perform() async throws -> some IntentResult {
        await WidgetIntentRunner.send([
            "action": "seek", "fraction": fraction, "expect": WidgetIntentRunner.expect(ratingKey, revision),
        ])
        return .result()
    }
}

struct ResumeIntent: AudioPlaybackIntent {
    static var title: LocalizedStringResource = "Resume"
    static var isDiscoverable = false

    func perform() async throws -> some IntentResult {
        await WidgetIntentRunner.send(["action": "resume"])
        return .result()
    }
}

struct RandomFavouriteAlbumIntent: AudioPlaybackIntent {
    static var title: LocalizedStringResource = "Random Favourite Album"
    static var isDiscoverable = false

    func perform() async throws -> some IntentResult {
        await WidgetIntentRunner.send(["action": "randomFavouriteAlbum"])
        return .result()
    }
}

struct ShuffleFavouriteSongsIntent: AudioPlaybackIntent {
    static var title: LocalizedStringResource = "Shuffle Favourite Songs"
    static var isDiscoverable = false

    func perform() async throws -> some IntentResult {
        await WidgetIntentRunner.send(["action": "shuffleFavouriteSongs"])
        return .result()
    }
}

struct RandomUnplayedAlbumIntent: AudioPlaybackIntent {
    static var title: LocalizedStringResource = "Random Unplayed Album"
    static var isDiscoverable = false

    func perform() async throws -> some IntentResult {
        await WidgetIntentRunner.send(["action": "randomUnplayedAlbum"])
        return .result()
    }
}
