import AppIntents
import Foundation

// App Intents / Siri surface for ramus.
//
// Two tiers cross the Rust↔Swift seam (the C ABI in libapp.a, declared in the
// bridging header):
//
//   * Read-only query + entity vocabulary — `perform()`/entity queries call
//     `ramus_siri_probe` / `ramus_siri_genres` / `ramus_siri_artists`, which
//     open a read-only view of the on-disk cache and answer offline, with no
//     dependency on the running app.
//   * Playback — `RamusPlayMusicIntent` calls `ramus_siri_play`, which reaches
//     the *live* player. It sets `openAppWhenRun` so the app is foregrounded
//     (the audio engine needs the running app), then starts the queue.
//
// Genres and artists are modelled as `AppEntity`s so the assistant can turn a
// spoken word ("post-hardcore", "Touché Amoré") into a concrete library item —
// resolved by the entity queries below. On iOS's semantic-assistant tier this
// lets Siri fill the play intent's parameters from the utterance directly; on
// any device the Shortcuts app fills them from `suggestedEntities()`.
//
// Discovery requires these types to be compiled into the app *target* (not the
// static-library plugin), so this file lives alongside `main.mm`.

// MARK: - Rust bridge

/// Minimal decode of a probe/play result (`ok` + a ready-to-speak `spoken`).
private struct RamusSpokenResult: Decodable {
    let ok: Bool
    let spoken: String
    let error: String?
}

/// Decode of a genre/artist listing (`{ok, items:[{name, …}]}`).
private struct RamusEntityList: Decodable {
    struct Item: Decodable { let name: String }
    let ok: Bool
    let items: [Item]
}

/// Decode of an album listing (`{ok, items:[{sourceId, title, artist}]}`).
private struct RamusAlbumListJSON: Decodable {
    struct Item: Decodable {
        let sourceId: String
        let title: String
        let artist: String
    }
    let ok: Bool
    let items: [Item]
}

/// A library album the assistant can play. `sourceId` (a rating key) is the
/// stable play target; `title`/`artist` are for display.
struct RamusAlbumRef {
    let sourceId: String
    let title: String
    let artist: String
}

/// Thin wrapper over the Rust C ABI. Always frees the returned buffer.
enum RamusLibraryBridge {
    /// snake_case Rust JSON (`album_count`, `track_count`) → camelCase Swift.
    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    /// Run an FFI call that returns a heap JSON C string, freeing it, and hand
    /// back the bytes.
    private static func callJSON(
        _ call: () -> UnsafeMutablePointer<CChar>?
    ) -> Data? {
        guard let raw = call() else { return nil }
        defer { ramus_siri_free(raw) }
        return String(cString: raw).data(using: .utf8)
    }

    /// Bridge an optional Swift string to a nullable C string for the duration
    /// of `body` (nil → null pointer).
    private static func withOptionalCString<R>(
        _ string: String?,
        _ body: (UnsafePointer<CChar>?) -> R
    ) -> R {
        if let string { return string.withCString { body($0) } }
        return body(nil)
    }

    // Read-only ---------------------------------------------------------------

    static func probe(genre: String?) -> String {
        let data = callJSON { withOptionalCString(genre) { ramus_siri_probe($0) } }
        guard let data, let result = try? decoder.decode(RamusSpokenResult.self, from: data) else {
            return "ramus returned an unexpected response."
        }
        return result.ok ? result.spoken : (result.error ?? "ramus couldn't answer that.")
    }

    static func genreNames(matching query: String?) -> [String] {
        entityNames(query) { withOptionalCString(query) { ramus_siri_genres($0) } }
    }

    static func artistNames(matching query: String?) -> [String] {
        entityNames(query) { withOptionalCString(query) { ramus_siri_artists($0) } }
    }

    static func albums(matching query: String?) -> [RamusAlbumRef] {
        guard
            let data = callJSON({ withOptionalCString(query) { ramus_siri_albums($0) } }),
            let list = try? decoder.decode(RamusAlbumListJSON.self, from: data),
            list.ok
        else { return [] }
        return list.items.map {
            RamusAlbumRef(sourceId: $0.sourceId, title: $0.title, artist: $0.artist)
        }
    }

    private static func entityNames(
        _ query: String?,
        _ call: () -> UnsafeMutablePointer<CChar>?
    ) -> [String] {
        guard
            let data = callJSON(call),
            let list = try? decoder.decode(RamusEntityList.self, from: data),
            list.ok
        else { return [] }
        return list.items.map(\.name)
    }

    // Playback ----------------------------------------------------------------

    static func play(genre: String?, artist: String?, album: String?) -> String {
        let data = callJSON {
            withOptionalCString(genre) { genrePtr in
                withOptionalCString(artist) { artistPtr in
                    withOptionalCString(album) { albumPtr in
                        ramus_siri_play(genrePtr, artistPtr, albumPtr)
                    }
                }
            }
        }
        guard let data, let result = try? decoder.decode(RamusSpokenResult.self, from: data) else {
            return "ramus couldn't start playback."
        }
        return result.ok ? result.spoken : (result.error ?? "ramus couldn't start playback.")
    }
}

// MARK: - Entities

/// A genre from the library. Its identifier is the genre name — playback and
/// resolution are name-based, so an id round-trips to an entity with no lookup.
struct RamusGenreEntity: AppEntity {
    static var typeDisplayRepresentation = TypeDisplayRepresentation(name: "Genre")
    static var defaultQuery = RamusGenreQuery()

    let id: String
    var name: String { id }

    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(id)") }

    init(_ name: String) { self.id = name }
}

/// An artist from the library. Identifier is the artist name (see the note on
/// `RamusGenreEntity`).
struct RamusArtistEntity: AppEntity {
    static var typeDisplayRepresentation = TypeDisplayRepresentation(name: "Artist")
    static var defaultQuery = RamusArtistQuery()

    let id: String
    var name: String { id }

    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(id)") }

    init(_ name: String) { self.id = name }
}

// MARK: - Entity queries

/// Resolves genre entities for Siri and Shortcuts. `EntityStringQuery` lets the
/// assistant match a spoken word to a stored genre tag.
struct RamusGenreQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [RamusGenreEntity] {
        // The identifier is the genre name, so rebuild directly — no lookup.
        identifiers.map(RamusGenreEntity.init)
    }

    func suggestedEntities() async throws -> [RamusGenreEntity] {
        RamusLibraryBridge.genreNames(matching: nil).map(RamusGenreEntity.init)
    }
}

extension RamusGenreQuery: EntityStringQuery {
    func entities(matching string: String) async throws -> [RamusGenreEntity] {
        RamusLibraryBridge.genreNames(matching: string).map(RamusGenreEntity.init)
    }
}

struct RamusArtistQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [RamusArtistEntity] {
        identifiers.map(RamusArtistEntity.init)
    }

    func suggestedEntities() async throws -> [RamusArtistEntity] {
        RamusLibraryBridge.artistNames(matching: nil).map(RamusArtistEntity.init)
    }
}

extension RamusArtistQuery: EntityStringQuery {
    func entities(matching string: String) async throws -> [RamusArtistEntity] {
        RamusLibraryBridge.artistNames(matching: string).map(RamusArtistEntity.init)
    }
}

// MARK: - Intents

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

/// "Play some post-hardcore" / "play some of their music" — starts playback of a
/// genre or an artist. Foregrounds the app because the audio engine lives in the
/// running process.
struct RamusPlayMusicIntent: AppIntent {
    static var title: LocalizedStringResource = "Play music in ramus"
    static var description = IntentDescription(
        "Play a genre or an artist from your ramus library."
    )

    // Playback needs the live player (mpv bridge), so bring the app forward.
    static var openAppWhenRun: Bool = true

    @Parameter(title: "Genre", description: "A genre to play, e.g. post-hardcore.")
    var genre: RamusGenreEntity?

    @Parameter(title: "Artist", description: "An artist to play.")
    var artist: RamusArtistEntity?

    func perform() async throws -> some IntentResult & ProvidesDialog {
        let spoken = RamusLibraryBridge.play(genre: genre?.name, artist: artist?.name, album: nil)
        return .result(dialog: IntentDialog(stringLiteral: spoken))
    }
}

// MARK: - App Shortcuts

/// Registers the voice phrases. Every phrase must contain the app name. The
/// phrases are deliberately un-parameterised: a spoken genre/artist is filled by
/// the semantic-assistant tier from the utterance, or by the Shortcuts app from
/// each entity's `suggestedEntities()`. (Parameterised phrases would require the
/// entity queries to be `EnumerableEntityQuery`, which a live library can't be.)
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
        // The "play …" phrases exist only on the phrase tier (iOS 16–26). On the
        // iOS 27 build the `.audio.playAudio` assistant schema owns "play X in
        // ramus" and fills the genre/artist from the utterance; a competing
        // explicit App Shortcut phrase would out-rank the semantic tier and, being
        // un-parameterised, silently drop the spoken word ("play some post-hardcore"
        // → matches "play something" → empty genre). So it's excluded there.
        #if !RAMUS_IOS27_SCHEMAS
        AppShortcut(
            intent: RamusPlayMusicIntent(),
            phrases: [
                "Play music in \(.applicationName)",
                "Play something in \(.applicationName)"
            ],
            shortTitle: "Play music",
            systemImageName: "play.circle"
        )
        #endif
    }
}

// MARK: - iOS 27 semantic tier (assistant schema)
//
// Gated behind the RAMUS_IOS27_SCHEMAS build condition: the `.audio.*` assistant
// schemas are iOS-27-SDK symbols, so this block only compiles under Xcode 27
// beta (see scripts/ios-dev-ai.sh). Normal Xcode-26 builds leave the condition
// unset and skip it, keeping the iOS 16–26 tier above buildable.
//
// Where the tier above routes via App Shortcut *phrases*, this routes via the
// audio assistant schema: adopting `@AppIntent(schema: .audio.playAudio)` tells
// the semantic Siri "this is ramus's play action", so an utterance like "play
// some post-hardcore" can reach it phrase-free and fill the item to play from
// the words — the frontier we're testing. It reuses the exact same Rust play
// seam (`ramus_siri_play`) as the phrase tier.
#if RAMUS_IOS27_SCHEMAS

/// A playable set of songs, keyed by what produced it. The audio schema has no
/// "genre" entity, so a genre and an artist both surface as a `songCollection`
/// (a bunch of songs) — the id records which, so `perform()` can route back to
/// the right Rust query.
enum RamusPlayTarget {
    case genre(String)
    case artist(String)
    case album(sourceId: String, title: String)

    init?(id: String) {
        if id.hasPrefix("genre:") {
            self = .genre(String(id.dropFirst("genre:".count)))
        } else if id.hasPrefix("artist:") {
            self = .artist(String(id.dropFirst("artist:".count)))
        } else if id.hasPrefix("album:") {
            // "album:<sourceId>|<title>" — the source id (a rating key) never
            // contains "|", so split on the first one; the title (which might)
            // is the remainder.
            let rest = String(id.dropFirst("album:".count))
            if let sep = rest.firstIndex(of: "|") {
                self = .album(
                    sourceId: String(rest[..<sep]),
                    title: String(rest[rest.index(after: sep)...])
                )
            } else {
                self = .album(sourceId: rest, title: rest)
            }
        } else {
            return nil
        }
    }

    var name: String {
        switch self {
        case .genre(let name), .artist(let name): return name
        case .album(_, let title): return title
        }
    }
}

/// A `songCollection` schema entity (`.audio.songCollection`) standing in for a
/// genre or an artist. `title` is a schema property (the macro wraps it); the id
/// encodes the source (`genre:…` / `artist:…`).
@available(iOS 27.0, *)
@AppEntity(schema: .audio.songCollection)
struct RamusSongCollectionEntity {
    static var defaultQuery = RamusSongCollectionQuery()
    static var typeDisplayRepresentation = TypeDisplayRepresentation(name: "Music")

    let id: String
    // The schema requires this property to be optional.
    var title: String?

    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(title ?? id)") }

    init(id: String, title: String) {
        self.id = id
        self.title = title
    }

    init(genre name: String) {
        self.init(id: "genre:\(name)", title: name)
    }

    init(artist name: String) {
        self.init(id: "artist:\(name)", title: name)
    }

    init(albumSourceId: String, title: String) {
        self.init(id: "album:\(albumSourceId)|\(title)", title: title)
    }
}

/// Resolves song-collection entities. `EntityStringQuery` is what lets the
/// assistant turn a spoken word into a collection: a query surfaces both any
/// matching genre and any matching artist, so "post-hardcore" and "Touché Amoré"
/// each resolve to something playable.
@available(iOS 27.0, *)
struct RamusSongCollectionQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [RamusSongCollectionEntity] {
        // The id carries the name, so rebuild directly.
        identifiers.map { id in
            RamusSongCollectionEntity(id: id, title: RamusPlayTarget(id: id)?.name ?? id)
        }
    }

    func suggestedEntities() async throws -> [RamusSongCollectionEntity] {
        let genres = RamusLibraryBridge.genreNames(matching: nil).prefix(20)
            .map(RamusSongCollectionEntity.init(genre:))
        let artists = RamusLibraryBridge.artistNames(matching: nil).prefix(20)
            .map(RamusSongCollectionEntity.init(artist:))
        let albums = RamusLibraryBridge.albums(matching: nil).prefix(20)
            .map { RamusSongCollectionEntity(albumSourceId: $0.sourceId, title: $0.title) }
        return genres + artists + albums
    }
}

@available(iOS 27.0, *)
extension RamusSongCollectionQuery: EntityStringQuery {
    func entities(matching string: String) async throws -> [RamusSongCollectionEntity] {
        let genres = RamusLibraryBridge.genreNames(matching: string)
            .map(RamusSongCollectionEntity.init(genre:))
        let artists = RamusLibraryBridge.artistNames(matching: string)
            .map(RamusSongCollectionEntity.init(artist:))
        let albums = RamusLibraryBridge.albums(matching: string)
            .map { RamusSongCollectionEntity(albumSourceId: $0.sourceId, title: $0.title) }
        let all = genres + artists + albums
        // Each category already prefers an exact SQL match, but across categories
        // a spoken "metal" still pulls the Metal genre AND metal-ish artists /
        // albums. When anything matches the words exactly, resolve to just those
        // so the assistant isn't handed an ambiguous pile.
        let exact = all.filter { ($0.title ?? "").caseInsensitiveCompare(string) == .orderedSame }
        return exact.isEmpty ? all : exact
    }
}

/// The `playAudio` schema types `audioEntity` as a union of the audio entity
/// kinds. ramus only offers song collections, so this union has the one case.
@available(iOS 27.0, *)
@UnionValue
enum RamusAudioItem {
    case songCollection(RamusSongCollectionEntity)
}

/// `playbackAttributes` schema enum — a required `playAudio` parameter. ramus
/// ignores it (it always plays normally), but the schema demands the property.
@available(iOS 27.0, *)
@AppEnum(schema: .audio.playbackAttributes)
enum RamusPlaybackAttributes: String {
    case shuffle
    case `repeat`

    static var caseDisplayRepresentations: [RamusPlaybackAttributes: DisplayRepresentation] {
        [.shuffle: "Shuffle", .repeat: "Repeat"]
    }
}

/// `queueInsertionLocation` schema enum — an optional `playAudio` parameter. The
/// schema requires the `next` and `tail` cases.
@available(iOS 27.0, *)
@AppEnum(schema: .audio.queueInsertionLocation)
enum RamusQueueLocation: String {
    case next
    case tail

    static var caseDisplayRepresentations: [RamusQueueLocation: DisplayRepresentation] {
        [.next: "Next", .tail: "Later"]
    }
}

/// `warmupAudioQueueResult` schema entity — an optional `playAudio` parameter.
/// ramus doesn't pre-warm queues, so this is only ever nil, but the schema
/// requires the (optional) property to exist with a conforming type.
@available(iOS 27.0, *)
struct RamusWarmupResultQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [RamusWarmupResult] { [] }
    func suggestedEntities() async throws -> [RamusWarmupResult] { [] }
}

// ramus never produces a warmup result, so the query is always empty; the schema
// just requires a resolution mechanism to exist.
@available(iOS 27.0, *)
extension RamusWarmupResultQuery: EntityStringQuery {
    func entities(matching string: String) async throws -> [RamusWarmupResult] { [] }
}

@available(iOS 27.0, *)
@AppEntity(schema: .audio.warmupAudioQueueResult)
struct RamusWarmupResult {
    static var defaultQuery = RamusWarmupResultQuery()
    static var typeDisplayRepresentation = TypeDisplayRepresentation(name: "Warmup")

    let id: String

    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(id)") }
}

/// The pure phrase-free play action. `@AppIntent(schema: .audio.playAudio)` also
/// makes it conform to `AudioPlaybackIntent`; `openAppWhenRun` still foregrounds
/// the app because ramus's audio engine needs the running process. The parameter
/// set is the schema's canonical shape (the macro injects `@Parameter`); ramus
/// only acts on `audioEntity`.
@available(iOS 27.0, *)
@AppIntent(schema: .audio.playAudio)
struct RamusPlayAudioIntent {
    static var openAppWhenRun: Bool = true

    var audioEntity: RamusAudioItem
    var playbackAttributes: Set<RamusPlaybackAttributes>
    var warmupAudioQueueResult: RamusWarmupResult?
    var queueLocation: RamusQueueLocation?

    func perform() async throws -> some IntentResult & ProvidesDialog {
        let collection: RamusSongCollectionEntity
        switch audioEntity {
        case .songCollection(let entity):
            collection = entity
        }

        let spoken: String
        switch RamusPlayTarget(id: collection.id) {
        case .genre(let name):
            spoken = RamusLibraryBridge.play(genre: name, artist: nil, album: nil)
        case .artist(let name):
            spoken = RamusLibraryBridge.play(genre: nil, artist: name, album: nil)
        case .album(let sourceId, _):
            spoken = RamusLibraryBridge.play(genre: nil, artist: nil, album: sourceId)
        case .none:
            // Fall back to treating the display title as an artist.
            spoken = RamusLibraryBridge.play(genre: nil, artist: collection.title, album: nil)
        }
        return .result(dialog: IntentDialog(stringLiteral: spoken))
    }
}

#endif
