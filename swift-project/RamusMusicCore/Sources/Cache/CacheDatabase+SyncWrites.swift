import Foundation
import GRDB
import Models

extension CacheDatabase {

    // MARK: - Upsert: Artists

    /// Insert or update an artist by sourceId. Returns the row id.
    @discardableResult
    func upsertArtist(
        name: String, sortName: String?, sourceId: String,
        artUrl: String?, summary: String?, updatedAt: Int?
    ) throws -> Int64 {
        try dbPool.write { db in
            try db.execute(
                sql: """
                    INSERT INTO artists (name, sortName, sourceId, artUrl, summary, updatedAt)
                    VALUES (?, ?, ?, ?, ?, ?)
                    ON CONFLICT(sourceId) DO UPDATE SET
                        name = excluded.name, sortName = excluded.sortName,
                        artUrl = excluded.artUrl, summary = excluded.summary,
                        updatedAt = excluded.updatedAt
                    """,
                arguments: [name, sortName, sourceId, artUrl, summary, updatedAt]
            )
            let row = try Row.fetchOne(db, sql: "SELECT id FROM artists WHERE sourceId = ?", arguments: [sourceId])
            return row!["id"]
        }
    }

    // MARK: - Upsert: Albums

    @discardableResult
    func upsertAlbum(
        title: String, artistId: Int64, year: Int?,
        sourceId: String, artUrl: String?,
        rating: Double? = nil, studio: String? = nil,
        updatedAt: Int?,
        addedAt: Int? = nil, lastViewedAt: Int? = nil
    ) throws -> Int64 {
        try dbPool.write { db in
            try db.execute(
                sql: """
                    INSERT INTO albums (title, artistId, year, sourceId, artUrl, rating, studio, updatedAt, addedAt, lastViewedAt)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(sourceId) DO UPDATE SET
                        title = excluded.title, artistId = excluded.artistId,
                        year = excluded.year, artUrl = excluded.artUrl,
                        rating = COALESCE(excluded.rating, albums.rating),
                        studio = COALESCE(excluded.studio, albums.studio),
                        updatedAt = excluded.updatedAt,
                        addedAt = COALESCE(excluded.addedAt, albums.addedAt),
                        lastViewedAt = COALESCE(excluded.lastViewedAt, albums.lastViewedAt)
                    """,
                arguments: [title, artistId, year, sourceId, artUrl, rating, studio, updatedAt, addedAt, lastViewedAt]
            )
            let row = try Row.fetchOne(db, sql: "SELECT id FROM albums WHERE sourceId = ?", arguments: [sourceId])
            return row!["id"]
        }
    }

    // MARK: - Upsert: Tracks

    @discardableResult
    func upsertTrack(
        title: String, albumId: Int64, artistId: Int64,
        trackNumber: Int?, discNumber: Int?, durationMs: Int?,
        sourceId: String, codec: String?,
        partKey: String?, streamId: Int?,
        userRating: Double? = nil, bitrate: Int? = nil,
        trackArtist: String? = nil,
        updatedAt: Int?
    ) throws -> Int64 {
        try dbPool.write { db in
            try db.execute(
                sql: """
                    INSERT INTO tracks (title, albumId, artistId, trackNumber, discNumber,
                        durationMs, sourceId, codec, partKey, streamId,
                        userRating, bitrate, trackArtist, updatedAt)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT(sourceId) DO UPDATE SET
                        title = excluded.title, albumId = excluded.albumId,
                        artistId = excluded.artistId, trackNumber = excluded.trackNumber,
                        discNumber = excluded.discNumber, durationMs = excluded.durationMs,
                        codec = excluded.codec, partKey = excluded.partKey,
                        streamId = excluded.streamId, userRating = excluded.userRating,
                        bitrate = excluded.bitrate, trackArtist = excluded.trackArtist,
                        updatedAt = excluded.updatedAt
                    """,
                arguments: [title, albumId, artistId, trackNumber, discNumber, durationMs, sourceId, codec, partKey, streamId, userRating, bitrate, trackArtist, updatedAt]
            )
            let row = try Row.fetchOne(db, sql: "SELECT id FROM tracks WHERE sourceId = ?", arguments: [sourceId])
            return row!["id"]
        }
    }

    // MARK: - Upsert: Genres

    /// Insert or get a genre by name (case-insensitive). Returns the row id.
    @discardableResult
    func upsertGenre(name: String) throws -> Int64 {
        try dbPool.write { db in
            if let existing = try Row.fetchOne(db, sql: "SELECT id FROM genres WHERE name = ? COLLATE NOCASE", arguments: [name]) {
                return existing["id"]
            }
            try db.execute(sql: "INSERT INTO genres (name) VALUES (?)", arguments: [name])
            return db.lastInsertedRowID
        }
    }

    /// Link an album to a genre. Ignores duplicates.
    func linkAlbumGenre(albumId: Int64, genreId: Int64) throws {
        try dbPool.write { db in
            try db.execute(
                sql: "INSERT OR IGNORE INTO album_genres (albumId, genreId) VALUES (?, ?)",
                arguments: [albumId, genreId]
            )
        }
    }

    /// Replace all genre links for an album.
    func setAlbumGenres(albumId: Int64, genreIds: [Int64]) throws {
        try dbPool.write { db in
            try db.execute(sql: "DELETE FROM album_genres WHERE albumId = ?", arguments: [albumId])
            for genreId in genreIds {
                try db.execute(
                    sql: "INSERT INTO album_genres (albumId, genreId) VALUES (?, ?)",
                    arguments: [albumId, genreId]
                )
            }
        }
    }

    // MARK: - Album Metadata Updates (Deep Sync)

    /// Update all deep-sync metadata for an album in a single write transaction:
    /// genre links, rating, studio, and UltraBlur colors.
    func updateAlbumDeepMetadata(
        albumId: Int64, genreIds: [Int64],
        rating: Double?, studio: String?, colorsJSON: String?
    ) throws {
        try dbPool.write { db in
            // Replace genre links (skip if Plex returned no genres — don't wipe existing)
            if !genreIds.isEmpty {
                try db.execute(sql: "DELETE FROM album_genres WHERE albumId = ?", arguments: [albumId])
                for genreId in genreIds {
                    try db.execute(
                        sql: "INSERT INTO album_genres (albumId, genreId) VALUES (?, ?)",
                        arguments: [albumId, genreId]
                    )
                }
            }

            // Update rating, studio, and colors
            try db.execute(
                sql: "UPDATE albums SET rating = ?, studio = ?, ultraBlurColors = ? WHERE id = ?",
                arguments: [rating, studio, colorsJSON, albumId]
            )
        }
    }

    // MARK: - Batch Upserts

    /// Batch upsert artists in a single transaction. Returns sourceId → row ID map.
    func batchUpsertArtists(_ items: [(name: String, sortName: String?, sourceId: String,
                                       artUrl: String?, summary: String?, updatedAt: Int?)]) throws -> [String: Int64] {
        try dbPool.write { db in
            var map: [String: Int64] = [:]
            for item in items {
                try db.execute(
                    sql: """
                        INSERT INTO artists (name, sortName, sourceId, artUrl, summary, updatedAt)
                        VALUES (?, ?, ?, ?, ?, ?)
                        ON CONFLICT(sourceId) DO UPDATE SET
                            name = excluded.name, sortName = excluded.sortName,
                            artUrl = excluded.artUrl, summary = excluded.summary,
                            updatedAt = excluded.updatedAt
                        """,
                    arguments: [item.name, item.sortName, item.sourceId, item.artUrl, item.summary, item.updatedAt]
                )
                let row = try Row.fetchOne(db, sql: "SELECT id FROM artists WHERE sourceId = ?", arguments: [item.sourceId])
                map[item.sourceId] = row!["id"]
            }
            return map
        }
    }

    /// Batch upsert albums in a single transaction. Returns sourceId → row ID map.
    func batchUpsertAlbums(_ items: [(title: String, artistId: Int64, year: Int?,
                                      sourceId: String, artUrl: String?, rating: Double?,
                                      studio: String?, updatedAt: Int?,
                                      addedAt: Int?, lastViewedAt: Int?)]) throws -> [String: Int64] {
        try dbPool.write { db in
            var map: [String: Int64] = [:]
            for item in items {
                try db.execute(
                    sql: """
                        INSERT INTO albums (title, artistId, year, sourceId, artUrl, rating, studio, updatedAt, addedAt, lastViewedAt)
                        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                        ON CONFLICT(sourceId) DO UPDATE SET
                            title = excluded.title, artistId = excluded.artistId,
                            year = excluded.year, artUrl = excluded.artUrl,
                            rating = COALESCE(excluded.rating, albums.rating),
                            studio = COALESCE(excluded.studio, albums.studio),
                            updatedAt = excluded.updatedAt,
                            addedAt = COALESCE(excluded.addedAt, albums.addedAt),
                            lastViewedAt = COALESCE(excluded.lastViewedAt, albums.lastViewedAt)
                        """,
                    arguments: [item.title, item.artistId, item.year, item.sourceId, item.artUrl,
                                item.rating, item.studio, item.updatedAt, item.addedAt, item.lastViewedAt]
                )
                let row = try Row.fetchOne(db, sql: "SELECT id FROM albums WHERE sourceId = ?", arguments: [item.sourceId])
                map[item.sourceId] = row!["id"]
            }
            return map
        }
    }

    /// Batch upsert tracks in a single transaction.
    func batchUpsertTracks(_ items: [(title: String, albumId: Int64, artistId: Int64,
                                      trackNumber: Int?, discNumber: Int?, durationMs: Int?,
                                      sourceId: String, codec: String?, partKey: String?,
                                      streamId: Int?, userRating: Double?, bitrate: Int?,
                                      trackArtist: String?, updatedAt: Int?)]) throws {
        try dbPool.write { db in
            for item in items {
                try db.execute(
                    sql: """
                        INSERT INTO tracks (title, albumId, artistId, trackNumber, discNumber,
                            durationMs, sourceId, codec, partKey, streamId,
                            userRating, bitrate, trackArtist, updatedAt)
                        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                        ON CONFLICT(sourceId) DO UPDATE SET
                            title = excluded.title, albumId = excluded.albumId,
                            artistId = excluded.artistId, trackNumber = excluded.trackNumber,
                            discNumber = excluded.discNumber, durationMs = excluded.durationMs,
                            codec = excluded.codec, partKey = excluded.partKey,
                            streamId = excluded.streamId, userRating = excluded.userRating,
                            bitrate = excluded.bitrate, trackArtist = excluded.trackArtist,
                            updatedAt = excluded.updatedAt
                        """,
                    arguments: [item.title, item.albumId, item.artistId, item.trackNumber, item.discNumber,
                                item.durationMs, item.sourceId, item.codec, item.partKey, item.streamId,
                                item.userRating, item.bitrate, item.trackArtist, item.updatedAt]
                )
            }
        }
    }

    /// Batch upsert genres and link them to albums in a single transaction.
    func batchUpsertGenresAndLinks(_ items: [(albumId: Int64, genreName: String)]) throws {
        guard !items.isEmpty else { return }
        try dbPool.write { db in
            for item in items {
                // Upsert genre
                let genreId: Int64
                if let existing = try Row.fetchOne(db, sql: "SELECT id FROM genres WHERE name = ? COLLATE NOCASE",
                                                   arguments: [item.genreName]) {
                    genreId = existing["id"]
                } else {
                    try db.execute(sql: "INSERT INTO genres (name) VALUES (?)", arguments: [item.genreName])
                    genreId = db.lastInsertedRowID
                }
                // Link
                try db.execute(
                    sql: "INSERT OR IGNORE INTO album_genres (albumId, genreId) VALUES (?, ?)",
                    arguments: [item.albumId, genreId]
                )
            }
        }
    }

    // MARK: - Bulk Timestamp Lookups

    struct CachedItemInfo: Sendable {
        let id: Int64
        let updatedAt: Int?
    }

    /// Load all artist sourceId → (id, updatedAt) in a single query.
    func allArtistTimestamps() throws -> [String: CachedItemInfo] {
        try dbPool.read { db in
            var map: [String: CachedItemInfo] = [:]
            let rows = try Row.fetchAll(db, sql: "SELECT sourceId, id, updatedAt FROM artists")
            for row in rows {
                let sourceId: String = row["sourceId"]
                map[sourceId] = CachedItemInfo(id: row["id"], updatedAt: row["updatedAt"])
            }
            return map
        }
    }

    struct CachedAlbumInfo: Sendable {
        let id: Int64
        let updatedAt: Int?
        let firstGenre: String?
    }

    /// Load all album sourceId → (id, updatedAt, firstGenre) in a single query.
    /// Includes the first linked genre name so we can detect genre-only edits
    /// (Plex doesn't always bump `updatedAt` for genre changes).
    func allAlbumTimestamps() throws -> [String: CachedAlbumInfo] {
        try dbPool.read { db in
            var map: [String: CachedAlbumInfo] = [:]
            let rows = try Row.fetchAll(db, sql: """
                SELECT a.sourceId, a.id, a.updatedAt,
                       (SELECT g.name FROM album_genres ag
                        JOIN genres g ON g.id = ag.genreId
                        WHERE ag.albumId = a.id
                        ORDER BY ag.rowid LIMIT 1) AS firstGenre
                FROM albums a
                """)
            for row in rows {
                let sourceId: String = row["sourceId"]
                map[sourceId] = CachedAlbumInfo(id: row["id"], updatedAt: row["updatedAt"], firstGenre: row["firstGenre"])
            }
            return map
        }
    }

    /// Load all track sourceId → (id, updatedAt) in a single query.
    func allTrackTimestamps() throws -> [String: CachedItemInfo] {
        try dbPool.read { db in
            var map: [String: CachedItemInfo] = [:]
            let rows = try Row.fetchAll(db, sql: "SELECT sourceId, id, updatedAt FROM tracks")
            for row in rows {
                let sourceId: String = row["sourceId"]
                map[sourceId] = CachedItemInfo(id: row["id"], updatedAt: row["updatedAt"])
            }
            return map
        }
    }

    // MARK: - ID Lookups (Sync)

    /// Lookup artist row ID by sourceId.
    func artistId(forSourceId sourceId: String) throws -> Int64? {
        try dbPool.read { db in
            let row = try Row.fetchOne(db, sql: "SELECT id FROM artists WHERE sourceId = ?", arguments: [sourceId])
            return row?["id"]
        }
    }

    /// Lookup album row ID by sourceId.
    func albumId(forSourceId sourceId: String) throws -> Int64? {
        try dbPool.read { db in
            let row = try Row.fetchOne(db, sql: "SELECT id FROM albums WHERE sourceId = ?", arguments: [sourceId])
            return row?["id"]
        }
    }

    /// Get the updatedAt timestamp for an album by sourceId.
    func albumUpdatedAt(sourceId: String) throws -> Int? {
        try dbPool.read { db in
            let row = try Row.fetchOne(db, sql: "SELECT updatedAt FROM albums WHERE sourceId = ?", arguments: [sourceId])
            return row?["updatedAt"]
        }
    }

}
