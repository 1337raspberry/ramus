import AppIntents
import Foundation

// App Intents / Siri surface for ramus.
//
// This is the spike that proves library data can cross from the Rust core into
// the assistant surface: `perform()` calls the C ABI in libapp.a
// (`ramus_siri_probe`, declared in the bridging header), which opens a
// read-only view of the on-disk cache and answers offline. No dependency on the
// running app's state — the intent works even on a cold launch.
//
// Discovery requires these types to be compiled into the app *target* (not the
// static-library plugin), so the file lives alongside `main.mm`.

/// Minimal decode of `ramus_core::siri_probe::ProbeResult`; extra JSON fields
/// are ignored.
private struct RamusProbeResult: Decodable {
    let ok: Bool
    let spoken: String
    let error: String?
}

/// Thin wrapper over the Rust C ABI. Always frees the returned buffer.
enum RamusLibraryBridge {
    static func probe(genre: String?) -> String {
        let raw: UnsafeMutablePointer<CChar>?
        if let genre {
            raw = genre.withCString { ramus_siri_probe($0) }
        } else {
            raw = ramus_siri_probe(nil)
        }

        guard let raw else { return "ramus couldn't read your library." }
        defer { ramus_siri_free(raw) }

        let json = String(cString: raw)
        guard
            let data = json.data(using: .utf8),
            let result = try? JSONDecoder().decode(RamusProbeResult.self, from: data)
        else {
            return "ramus returned an unexpected response."
        }
        return result.ok ? result.spoken : (result.error ?? "ramus couldn't answer that.")
    }
}

/// "What have I listened to in ramus?" — optionally filtered by a genre.
struct RamusMusicQueryIntent: AppIntent {
    static var title: LocalizedStringResource = "Find music in ramus"
    static var description = IntentDescription(
        "Ask ramus about your library and what you've listened to recently."
    )

    // Run in-process without foregrounding the UI so Siri speaks the answer.
    static var openAppWhenRun: Bool = false

    @Parameter(title: "Genre", description: "Optional genre to focus on, e.g. post-hardcore.")
    var genre: String?

    func perform() async throws -> some IntentResult & ProvidesDialog {
        let spoken = RamusLibraryBridge.probe(genre: genre)
        return .result(dialog: IntentDialog(stringLiteral: spoken))
    }
}

/// Registers the voice phrases. Every phrase must contain the app name; the
/// genre is supplied via the Shortcuts UI (free-text voice parameters resolve
/// reliably only on the newer semantic-assistant tier).
struct RamusAppShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: RamusMusicQueryIntent(),
            phrases: [
                "What have I listened to in \(.applicationName)",
                "What's in my \(.applicationName) library",
                "Ask \(.applicationName) about my music"
            ],
            shortTitle: "Find music",
            systemImageName: "music.note.list"
        )
    }
}
