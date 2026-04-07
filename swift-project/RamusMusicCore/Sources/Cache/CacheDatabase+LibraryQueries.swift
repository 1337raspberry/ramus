import Foundation
import GRDB
import Models

extension CacheDatabase {

    // MARK: - Genre Queries

    /// Genre name → set of album IDs. Used for deduplicated subtree counting.
    public func genreAlbumSets() throws -> [String: Set<Int64>] {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT g.name, ag.albumId
                FROM genres g
                JOIN album_genres ag ON ag.genreId = g.id
                """)
            var result: [String: Set<Int64>] = [:]
            for row in rows {
                let name: String = row["name"]
                let albumId: Int64 = row["albumId"]
                result[name, default: []].insert(albumId)
            }
            return result
        }
    }

    /// Genre name → set of album IDs, filtered to favourite albums only.
    public func favouriteAlbumGenreSets() throws -> [String: Set<Int64>] {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT g.name, ag.albumId
                FROM genres g
                JOIN album_genres ag ON ag.genreId = g.id
                JOIN albums a ON a.id = ag.albumId
                WHERE a.rating >= 10
                """)
            var result: [String: Set<Int64>] = [:]
            for row in rows {
                let name: String = row["name"]
                let albumId: Int64 = row["albumId"]
                result[name, default: []].insert(albumId)
            }
            return result
        }
    }

    /// Genre IDs by names (case-insensitive). Used to resolve genre node names to IDs for querying.
    public func genreIds(forNames names: [String]) throws -> [Int64] {
        guard !names.isEmpty else { return [] }
        let placeholders = Self.sqlPlaceholders(count: names.count)
        return try dbPool.read { db in
            let args = StatementArguments(names)
            let rows = try Row.fetchAll(db, sql: """
                SELECT id FROM genres WHERE name IN (\(placeholders)) COLLATE NOCASE
                """, arguments: args)
            return rows.map { $0["id"] as Int64 }
        }
    }

    /// Genres linked to an album by sourceId.
    public func genresForAlbum(sourceId: String) throws -> [String] {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT g.name FROM genres g
                JOIN album_genres ag ON ag.genreId = g.id
                JOIN albums a ON a.id = ag.albumId
                WHERE a.sourceId = ?
                ORDER BY g.name
                """, arguments: [sourceId])
            return rows.map { $0["name"] as String }
        }
    }

    // MARK: - Album Queries

    /// Albums linked to ANY of the given genre IDs, deduplicated by album id,
    /// sorted by artist name then year.
    public func albumsByGenreIds(_ genreIds: [Int64]) throws -> [(id: Int64, title: String, artistName: String, year: Int?, artUrl: String?, sourceId: String, rating: Double?, studio: String?, addedAt: Int?, lastViewedAt: Int?)] {
        guard !genreIds.isEmpty else { return [] }
        let placeholders = Self.sqlPlaceholders(count: genreIds.count)
        return try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT DISTINCT a.id, a.title, ar.name AS artistName, a.year, a.artUrl, a.sourceId, a.rating, a.studio, a.addedAt, a.lastViewedAt
                FROM albums a
                JOIN album_genres ag ON ag.albumId = a.id
                JOIN artists ar ON ar.id = a.artistId
                WHERE ag.genreId IN (\(placeholders))
                ORDER BY ar.name COLLATE NOCASE, a.year
                """, arguments: StatementArguments(genreIds))
            return rows.map { (
                id: $0["id"], title: $0["title"], artistName: $0["artistName"],
                year: $0["year"], artUrl: $0["artUrl"], sourceId: $0["sourceId"],
                rating: $0["rating"], studio: $0["studio"],
                addedAt: $0["addedAt"], lastViewedAt: $0["lastViewedAt"]
            ) }
        }
    }

    /// Albums for a given artist sourceId, with artist name.
    public func albumsByArtistSourceId(_ sourceId: String) throws -> [(id: Int64, title: String, artistName: String, year: Int?, artUrl: String?, sourceId: String, rating: Double?, studio: String?, addedAt: Int?, lastViewedAt: Int?)] {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT a.id, a.title, ar.name AS artistName, a.year, a.artUrl, a.sourceId, a.rating, a.studio, a.addedAt, a.lastViewedAt
                FROM albums a
                JOIN artists ar ON ar.id = a.artistId
                WHERE ar.sourceId = ?
                ORDER BY a.year
                """, arguments: [sourceId])
            return rows.map { (
                id: $0["id"], title: $0["title"], artistName: $0["artistName"],
                year: $0["year"], artUrl: $0["artUrl"], sourceId: $0["sourceId"],
                rating: $0["rating"], studio: $0["studio"],
                addedAt: $0["addedAt"], lastViewedAt: $0["lastViewedAt"]
            ) }
        }
    }

    /// All albums with artist info, no limit.
    public func allAlbums() throws -> [(id: Int64, title: String, artistName: String, year: Int?, artUrl: String?, sourceId: String, rating: Double?, studio: String?, addedAt: Int?, lastViewedAt: Int?)] {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT a.id, a.title, ar.name AS artistName, a.year, a.artUrl, a.sourceId, a.rating, a.studio, a.addedAt, a.lastViewedAt
                FROM albums a
                JOIN artists ar ON ar.id = a.artistId
                ORDER BY ar.name COLLATE NOCASE, a.year
                """)
            return rows.map { (
                id: $0["id"], title: $0["title"], artistName: $0["artistName"],
                year: $0["year"], artUrl: $0["artUrl"], sourceId: $0["sourceId"],
                rating: $0["rating"], studio: $0["studio"],
                addedAt: $0["addedAt"], lastViewedAt: $0["lastViewedAt"]
            ) }
        }
    }

    /// All favourite albums with artist info.
    public func allFavouriteAlbums() throws -> [(id: Int64, title: String, artistName: String, year: Int?, artUrl: String?, sourceId: String, rating: Double?, studio: String?, addedAt: Int?, lastViewedAt: Int?)] {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT a.id, a.title, ar.name AS artistName, a.year, a.artUrl, a.sourceId, a.rating, a.studio, a.addedAt, a.lastViewedAt
                FROM albums a
                JOIN artists ar ON ar.id = a.artistId
                WHERE a.rating >= 10
                ORDER BY ar.name COLLATE NOCASE, a.year
                """)
            return rows.map { (
                id: $0["id"], title: $0["title"], artistName: $0["artistName"],
                year: $0["year"], artUrl: $0["artUrl"], sourceId: $0["sourceId"],
                rating: $0["rating"], studio: $0["studio"],
                addedAt: $0["addedAt"], lastViewedAt: $0["lastViewedAt"]
            ) }
        }
    }

    /// A single random album, or nil if the table is empty.
    public func randomAlbum() throws -> (id: Int64, title: String, artistName: String, year: Int?, artUrl: String?, sourceId: String, rating: Double?, studio: String?, addedAt: Int?, lastViewedAt: Int?)? {
        try dbPool.read { db in
            let row = try Row.fetchOne(db, sql: """
                SELECT a.id, a.title, ar.name AS artistName, a.year, a.artUrl, a.sourceId, a.rating, a.studio, a.addedAt, a.lastViewedAt
                FROM albums a
                JOIN artists ar ON ar.id = a.artistId
                ORDER BY RANDOM()
                LIMIT 1
                """)
            guard let row else { return nil }
            return (
                id: row["id"], title: row["title"], artistName: row["artistName"],
                year: row["year"], artUrl: row["artUrl"], sourceId: row["sourceId"],
                rating: row["rating"], studio: row["studio"],
                addedAt: row["addedAt"], lastViewedAt: row["lastViewedAt"]
            )
        }
    }

    /// Lookup a single album by sourceId with artist info.
    public func albumForSourceId(_ sourceId: String) throws -> (id: Int64, title: String, artistName: String, year: Int?, artUrl: String?, sourceId: String, rating: Double?, studio: String?, addedAt: Int?, lastViewedAt: Int?, artistSourceId: String?)? {
        try dbPool.read { db in
            let row = try Row.fetchOne(db, sql: """
                SELECT a.id, a.title, ar.name AS artistName, ar.sourceId AS artistSourceId,
                       a.year, a.artUrl, a.sourceId, a.rating, a.studio, a.addedAt, a.lastViewedAt
                FROM albums a
                JOIN artists ar ON ar.id = a.artistId
                WHERE a.sourceId = ?
                """, arguments: [sourceId])
            guard let row else { return nil }
            return (
                id: row["id"], title: row["title"], artistName: row["artistName"],
                year: row["year"], artUrl: row["artUrl"], sourceId: row["sourceId"],
                rating: row["rating"], studio: row["studio"],
                addedAt: row["addedAt"], lastViewedAt: row["lastViewedAt"],
                artistSourceId: row["artistSourceId"]
            )
        }
    }

    /// Retrieve UltraBlur colors for an album by sourceId.
    public func albumColors(sourceId: String) throws -> UltraBlurColors? {
        try dbPool.read { db in
            let row = try Row.fetchOne(db, sql:
                "SELECT ultraBlurColors FROM albums WHERE sourceId = ?",
                arguments: [sourceId])
            guard let json: String = row?["ultraBlurColors"] else { return nil }
            guard let data = json.data(using: .utf8) else { return nil }
            return try? JSONDecoder().decode(UltraBlurColors.self, from: data)
        }
    }

    // MARK: - Track Queries

    /// Tracks for an album by sourceId, ordered by disc then track number.
    public func tracksForAlbum(sourceId: String) throws -> [(id: Int64, title: String, trackNumber: Int?, discNumber: Int?, durationMs: Int?, codec: String?, partKey: String?, sourceId: String, streamId: Int?, userRating: Double?, bitrate: Int?, trackArtist: String?)] {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT t.id, t.title, t.trackNumber, t.discNumber, t.durationMs, t.codec, t.partKey, t.sourceId, t.streamId, t.userRating, t.bitrate, t.trackArtist
                FROM tracks t
                JOIN albums a ON a.id = t.albumId
                WHERE a.sourceId = ?
                ORDER BY t.discNumber, t.trackNumber
                """, arguments: [sourceId])
            return rows.map { (
                id: $0["id"], title: $0["title"], trackNumber: $0["trackNumber"],
                discNumber: $0["discNumber"], durationMs: $0["durationMs"],
                codec: $0["codec"], partKey: $0["partKey"], sourceId: $0["sourceId"],
                streamId: $0["streamId"], userRating: $0["userRating"], bitrate: $0["bitrate"],
                trackArtist: $0["trackArtist"]
            ) }
        }
    }

    /// All tracks with userRating >= 10, joined with album and artist data for playback.
    public func allFavouriteTracks() throws -> [(
        id: Int64, title: String, trackNumber: Int?, discNumber: Int?,
        durationMs: Int?, codec: String?, partKey: String?,
        sourceId: String, streamId: Int?, userRating: Double?, bitrate: Int?,
        trackArtist: String?,
        albumTitle: String, artistName: String, albumSourceId: String, albumArtUrl: String?
    )] {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT t.id, t.title, t.trackNumber, t.discNumber, t.durationMs,
                       t.codec, t.partKey, t.sourceId, t.streamId, t.userRating, t.bitrate,
                       t.trackArtist,
                       a.title AS albumTitle, ar.name AS artistName,
                       a.sourceId AS albumSourceId, a.artUrl AS albumArtUrl
                FROM tracks t
                JOIN albums a ON a.id = t.albumId
                JOIN artists ar ON ar.id = a.artistId
                WHERE t.userRating >= 10
                ORDER BY ar.name COLLATE NOCASE, a.year, t.discNumber, t.trackNumber
                """)
            return rows.map { (
                id: $0["id"], title: $0["title"], trackNumber: $0["trackNumber"],
                discNumber: $0["discNumber"], durationMs: $0["durationMs"],
                codec: $0["codec"], partKey: $0["partKey"], sourceId: $0["sourceId"],
                streamId: $0["streamId"], userRating: $0["userRating"], bitrate: $0["bitrate"],
                trackArtist: $0["trackArtist"],
                albumTitle: $0["albumTitle"], artistName: $0["artistName"],
                albumSourceId: $0["albumSourceId"], albumArtUrl: $0["albumArtUrl"]
            ) }
        }
    }

    // MARK: - Artist Queries

    /// All artists sorted by name.
    public func allArtists() throws -> [(id: Int64, name: String, sourceId: String, artUrl: String?)] {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT id, name, sourceId, artUrl FROM artists
                ORDER BY COALESCE(sortName, name) COLLATE NOCASE
                """)
            return rows.map { (
                id: $0["id"], name: $0["name"],
                sourceId: $0["sourceId"], artUrl: $0["artUrl"]
            ) }
        }
    }

    // MARK: - Stats

    public struct CacheStats: Sendable {
        public let artistCount: Int
        public let albumCount: Int
        public let trackCount: Int
        public let genreCount: Int
    }

    public func stats() throws -> CacheStats {
        try dbPool.read { db in
            let artists = try Int.fetchOne(db, sql: "SELECT COUNT(*) FROM artists") ?? 0
            let albums = try Int.fetchOne(db, sql: "SELECT COUNT(*) FROM albums") ?? 0
            let tracks = try Int.fetchOne(db, sql: "SELECT COUNT(*) FROM tracks") ?? 0
            let genres = try Int.fetchOne(db, sql: "SELECT COUNT(*) FROM genres") ?? 0
            return CacheStats(artistCount: artists, albumCount: albums, trackCount: tracks, genreCount: genres)
        }
    }
}
