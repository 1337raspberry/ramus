import Foundation
import GRDB
import Models

/// SQLite cache database using GRDB with WAL mode.
/// Stores artists, albums, tracks, genres, and album-genre relationships.
public final class CacheDatabase: Sendable {

    let dbPool: DatabasePool

    // MARK: - Init

    /// Open or create the cache database at the default location.
    public init() throws {
        let url = Self.defaultDatabaseURL()
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(),
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        dbPool = try DatabasePool(path: url.path, configuration: Self.makeConfig())
        try migrate()
    }

    /// Open with a custom DatabasePool (for testing).
    public init(dbPool: DatabasePool) throws {
        self.dbPool = dbPool
        try migrate()
    }

    /// Temporary database for testing (file-backed, auto-deleted).
    static func temporary() throws -> CacheDatabase {
        let tempDir = FileManager.default.temporaryDirectory
        let path = tempDir.appendingPathComponent("ramus_test_\(UUID().uuidString).db").path
        let dbPool = try DatabasePool(path: path, configuration: makeConfig())
        return try CacheDatabase(dbPool: dbPool)
    }

    private static func makeConfig() -> Configuration {
        var config = Configuration()
        config.prepareDatabase { db in
            try db.execute(sql: "PRAGMA busy_timeout = 5000")
            try db.execute(sql: "PRAGMA synchronous = NORMAL")
        }
        return config
    }

    private static func defaultDatabaseURL() -> URL {
        let home = FileManager.default.homeDirectoryForCurrentUser
        return home
            .appendingPathComponent(".local/share/ramus", isDirectory: true)
            .appendingPathComponent("cache.db")
    }

    // MARK: - Migration

    private func migrate() throws {
        var migrator = DatabaseMigrator()
        migrator.eraseDatabaseOnSchemaChange = true

        migrator.registerMigration("v1_complete") { db in
            try db.create(table: "artists") { t in
                t.autoIncrementedPrimaryKey("id")
                t.column("name", .text).notNull()
                t.column("sortName", .text)
                t.column("sourceId", .text).notNull().unique()
                t.column("artUrl", .text)
                t.column("summary", .text)
                t.column("updatedAt", .integer)
            }

            try db.create(table: "albums") { t in
                t.autoIncrementedPrimaryKey("id")
                t.column("title", .text).notNull()
                t.column("artistId", .integer).notNull()
                    .references("artists", onDelete: .cascade)
                t.column("year", .integer)
                t.column("sourceId", .text).notNull().unique()
                t.column("artUrl", .text)
                t.column("updatedAt", .integer)
                t.column("rating", .double)
                t.column("studio", .text)
                t.column("ultraBlurColors", .text)
                t.column("addedAt", .integer)
                t.column("lastViewedAt", .integer)
            }

            try db.create(table: "tracks") { t in
                t.autoIncrementedPrimaryKey("id")
                t.column("title", .text).notNull()
                t.column("albumId", .integer).notNull()
                    .references("albums", onDelete: .cascade)
                t.column("artistId", .integer).notNull()
                    .references("artists", onDelete: .cascade)
                t.column("trackNumber", .integer)
                t.column("discNumber", .integer)
                t.column("durationMs", .integer)
                t.column("sourceId", .text).notNull().unique()
                t.column("codec", .text)
                t.column("partKey", .text)
                t.column("updatedAt", .integer)
                t.column("streamId", .integer)
                t.column("userRating", .double)
                t.column("bitrate", .integer)
                t.column("trackArtist", .text)
            }

            try db.create(table: "genres") { t in
                t.autoIncrementedPrimaryKey("id")
                t.column("name", .text).notNull().unique().collate(.nocase)
            }

            try db.create(table: "album_genres") { t in
                t.column("albumId", .integer).notNull()
                    .references("albums", onDelete: .cascade)
                t.column("genreId", .integer).notNull()
                    .references("genres", onDelete: .cascade)
                t.primaryKey(["albumId", "genreId"])
            }

            try db.create(virtualTable: "tracks_fts", using: FTS5()) { t in
                t.synchronize(withTable: "tracks")
                t.tokenizer = .unicode61()
                t.prefixes = [2, 3]
                t.column("title")
            }

            try db.create(index: "idx_artists_name", on: "artists", columns: ["name"])
            try db.create(index: "idx_albums_title", on: "albums", columns: ["title"])
            try db.create(index: "idx_tracks_albumId", on: "tracks", columns: ["albumId"])
            try db.create(index: "idx_album_genres_genreId", on: "album_genres", columns: ["genreId"])
        }

        try migrator.migrate(dbPool)
    }

    // MARK: - Helpers

    static func sqlPlaceholders(count: Int) -> String {
        Array(repeating: "?", count: count).joined(separator: ",")
    }
}
