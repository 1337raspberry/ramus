import Foundation
import GRDB
import Models

extension CacheDatabase {

    // MARK: - Text Search Helpers

    /// Escape LIKE wildcard characters so user input is matched literally.
    static func escapeLikePattern(_ query: String) -> String {
        query
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "%", with: "\\%")
            .replacingOccurrences(of: "_", with: "\\_")
    }

    /// Build a SQL IN clause from a set of IDs, returning sorted placeholders and args.
    private static func inClause(_ ids: Set<Int64>) -> (placeholders: String, args: [Int64]) {
        let sorted = ids.sorted()
        return (sqlPlaceholders(count: sorted.count), sorted)
    }

    // MARK: - Album Search

    /// Search albums by artist name (LIKE contains), optionally constrained to album IDs.
    public func searchAlbumsByArtist(
        query: String, albumIds: Set<Int64>?, limit: Int = 50
    ) throws -> [(albumId: Int64, albumSourceId: String, albumTitle: String, artistName: String, year: Int?, artUrl: String?, rating: Double?)] {
        try dbPool.read { db in
            let likePattern = "%\(Self.escapeLikePattern(query))%"
            var sql = """
                SELECT DISTINCT a.id AS albumId, a.sourceId AS albumSourceId, a.title AS albumTitle,
                       ar.name AS artistName, a.year, a.artUrl, a.rating
                FROM albums a
                JOIN artists ar ON ar.id = a.artistId
                WHERE ar.name LIKE ? ESCAPE '\\'
                """
            var args: [any DatabaseValueConvertible] = [likePattern]

            if let albumIds, !albumIds.isEmpty {
                let clause = Self.inClause(albumIds)
                sql += " AND a.id IN (\(clause.placeholders))"
                for id in clause.args { args.append(id) }
            }

            sql += " ORDER BY ar.name COLLATE NOCASE, a.year LIMIT ?"
            args.append(limit)

            let rows = try Row.fetchAll(db, sql: sql, arguments: StatementArguments(args))
            return rows.map { (
                albumId: $0["albumId"], albumSourceId: $0["albumSourceId"],
                albumTitle: $0["albumTitle"], artistName: $0["artistName"], year: $0["year"],
                artUrl: $0["artUrl"], rating: $0["rating"]
            ) }
        }
    }

    /// Search albums by album title (LIKE contains), optionally constrained to album IDs.
    public func searchAlbumsByTitle(
        query: String, albumIds: Set<Int64>?, limit: Int = 50
    ) throws -> [(albumId: Int64, albumSourceId: String, albumTitle: String, artistName: String, year: Int?, artUrl: String?, rating: Double?)] {
        try dbPool.read { db in
            let likePattern = "%\(Self.escapeLikePattern(query))%"
            var sql = """
                SELECT DISTINCT a.id AS albumId, a.sourceId AS albumSourceId, a.title AS albumTitle,
                       ar.name AS artistName, a.year, a.artUrl, a.rating
                FROM albums a
                JOIN artists ar ON ar.id = a.artistId
                WHERE a.title LIKE ? ESCAPE '\\'
                """
            var args: [any DatabaseValueConvertible] = [likePattern]

            if let albumIds, !albumIds.isEmpty {
                let clause = Self.inClause(albumIds)
                sql += " AND a.id IN (\(clause.placeholders))"
                for id in clause.args { args.append(id) }
            }

            sql += " ORDER BY a.title COLLATE NOCASE, a.year LIMIT ?"
            args.append(limit)

            let rows = try Row.fetchAll(db, sql: sql, arguments: StatementArguments(args))
            return rows.map { (
                albumId: $0["albumId"], albumSourceId: $0["albumSourceId"],
                albumTitle: $0["albumTitle"], artistName: $0["artistName"], year: $0["year"],
                artUrl: $0["artUrl"], rating: $0["rating"]
            ) }
        }
    }

    /// Search albums by artist name OR album title (LIKE contains) in a single query.
    /// Used for free-text search to avoid two separate DB round-trips.
    public func searchAlbumsByArtistOrTitle(
        query: String, albumIds: Set<Int64>?, limit: Int = 50
    ) throws -> [(albumId: Int64, albumSourceId: String, albumTitle: String, artistName: String, year: Int?, artUrl: String?, rating: Double?)] {
        try dbPool.read { db in
            let likePattern = "%\(Self.escapeLikePattern(query))%"
            var sql = """
                SELECT DISTINCT a.id AS albumId, a.sourceId AS albumSourceId, a.title AS albumTitle,
                       ar.name AS artistName, a.year, a.artUrl, a.rating
                FROM albums a
                JOIN artists ar ON ar.id = a.artistId
                WHERE (ar.name LIKE ? ESCAPE '\\' OR a.title LIKE ? ESCAPE '\\')
                """
            var args: [any DatabaseValueConvertible] = [likePattern, likePattern]

            if let albumIds, !albumIds.isEmpty {
                let clause = Self.inClause(albumIds)
                sql += " AND a.id IN (\(clause.placeholders))"
                for id in clause.args { args.append(id) }
            }

            sql += " ORDER BY ar.name COLLATE NOCASE, a.year LIMIT ?"
            args.append(limit)

            let rows = try Row.fetchAll(db, sql: sql, arguments: StatementArguments(args))
            return rows.map { (
                albumId: $0["albumId"], albumSourceId: $0["albumSourceId"],
                albumTitle: $0["albumTitle"], artistName: $0["artistName"], year: $0["year"],
                artUrl: $0["artUrl"], rating: $0["rating"]
            ) }
        }
    }

    /// Fetch all albums optionally constrained to album IDs (for filter-only queries).
    public func searchAlbums(
        albumIds: Set<Int64>?, limit: Int = 100, randomOrder: Bool = false
    ) throws -> [(albumId: Int64, albumSourceId: String, albumTitle: String, artistName: String, year: Int?, artUrl: String?, rating: Double?)] {
        try dbPool.read { db in
            var sql = """
                SELECT a.id AS albumId, a.sourceId AS albumSourceId, a.title AS albumTitle,
                       ar.name AS artistName, a.year, a.artUrl, a.rating
                FROM albums a
                JOIN artists ar ON ar.id = a.artistId
                """
            var args: [any DatabaseValueConvertible] = []

            if let albumIds, !albumIds.isEmpty {
                let clause = Self.inClause(albumIds)
                sql += " WHERE a.id IN (\(clause.placeholders))"
                for id in clause.args { args.append(id) }
            }

            sql += randomOrder ? " ORDER BY RANDOM() LIMIT ?" : " ORDER BY ar.name COLLATE NOCASE, a.year LIMIT ?"
            args.append(limit)

            let rows = try Row.fetchAll(db, sql: sql, arguments: StatementArguments(args))
            return rows.map { (
                albumId: $0["albumId"], albumSourceId: $0["albumSourceId"],
                albumTitle: $0["albumTitle"], artistName: $0["artistName"], year: $0["year"],
                artUrl: $0["artUrl"], rating: $0["rating"]
            ) }
        }
    }

    // MARK: - Track Search

    /// Enriched FTS5 search returning track + artist + album display data.
    /// Optionally constrained to a set of album IDs.
    public func searchTracksEnriched(
        ftsQuery: String, albumIds: Set<Int64>?, limit: Int = 50
    ) throws -> [(id: Int64, trackSourceId: String, trackTitle: String, artistName: String, albumTitle: String, albumSourceId: String, artUrl: String?, trackArtist: String?)] {
        try dbPool.read { db in
            guard !ftsQuery.isEmpty else { return [] }

            var sql = """
                SELECT t.id, t.sourceId AS trackSourceId, t.title AS trackTitle,
                       ar.name AS artistName, a.title AS albumTitle, a.sourceId AS albumSourceId,
                       a.artUrl, t.trackArtist
                FROM tracks t
                JOIN tracks_fts fts ON fts.rowid = t.id
                JOIN albums a ON a.id = t.albumId
                JOIN artists ar ON ar.id = t.artistId
                WHERE tracks_fts MATCH ?
                """
            var args: [any DatabaseValueConvertible] = [ftsQuery]

            if let albumIds, !albumIds.isEmpty {
                let clause = Self.inClause(albumIds)
                sql += " AND t.albumId IN (\(clause.placeholders))"
                for id in clause.args { args.append(id) }
            }

            sql += " LIMIT ?"
            args.append(limit)

            let rows = try Row.fetchAll(db, sql: sql, arguments: StatementArguments(args))
            return rows.map { (
                id: $0["id"], trackSourceId: $0["trackSourceId"],
                trackTitle: $0["trackTitle"], artistName: $0["artistName"],
                albumTitle: $0["albumTitle"], albumSourceId: $0["albumSourceId"],
                artUrl: $0["artUrl"], trackArtist: $0["trackArtist"]
            ) }
        }
    }

    /// Fetch composite search candidates for Fuse fuzzy fallback.
    public func searchCandidates(
        albumIds: Set<Int64>?, limit: Int = 5000
    ) throws -> [(id: Int64, trackSourceId: String, trackTitle: String, artistName: String, albumTitle: String, albumSourceId: String, artUrl: String?, trackArtist: String?)] {
        try dbPool.read { db in
            var sql = """
                SELECT t.id, t.sourceId AS trackSourceId, t.title AS trackTitle,
                       ar.name AS artistName, a.title AS albumTitle, a.sourceId AS albumSourceId,
                       a.artUrl, t.trackArtist
                FROM tracks t
                JOIN albums a ON a.id = t.albumId
                JOIN artists ar ON ar.id = t.artistId
                """
            var args: [any DatabaseValueConvertible] = []

            if let albumIds, !albumIds.isEmpty {
                let clause = Self.inClause(albumIds)
                sql += " WHERE t.albumId IN (\(clause.placeholders))"
                for id in clause.args { args.append(id) }
            }

            sql += " LIMIT ?"
            args.append(limit)

            let rows = try Row.fetchAll(db, sql: sql, arguments: StatementArguments(args))
            return rows.map { (
                id: $0["id"], trackSourceId: $0["trackSourceId"],
                trackTitle: $0["trackTitle"], artistName: $0["artistName"],
                albumTitle: $0["albumTitle"], albumSourceId: $0["albumSourceId"],
                artUrl: $0["artUrl"], trackArtist: $0["trackArtist"]
            ) }
        }
    }

    // MARK: - Filter Queries

    /// Album IDs matching a year range constraint.
    public func albumIdsForYearRange(op: RangeOp, value: Int) throws -> Set<Int64> {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT id FROM albums WHERE year \(op.sqlLiteral) ?
                """, arguments: [value])
            return Set(rows.map { $0["id"] as Int64 })
        }
    }

    /// Album IDs linked to any of the given genre names (case-insensitive).
    public func albumIdsForGenreNames(_ names: [String]) throws -> Set<Int64> {
        guard !names.isEmpty else { return [] }
        let genreIdList = try genreIds(forNames: names)
        guard !genreIdList.isEmpty else { return [] }
        let placeholders = Self.sqlPlaceholders(count: genreIdList.count)
        return try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT DISTINCT albumId FROM album_genres WHERE genreId IN (\(placeholders))
                """, arguments: StatementArguments(genreIdList))
            return Set(rows.map { $0["albumId"] as Int64 })
        }
    }

    /// Album IDs where the user has set a favourite rating (>= 10).
    public func albumIdsForFavourites() throws -> Set<Int64> {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: "SELECT id FROM albums WHERE rating >= 10")
            return Set(rows.map { $0["id"] as Int64 })
        }
    }

    /// Album IDs matching a rating range constraint.
    public func albumIdsForRatingRange(op: RangeOp, value: Double) throws -> Set<Int64> {
        try dbPool.read { db in
            let rows = try Row.fetchAll(db, sql: """
                SELECT id FROM albums WHERE rating \(op.sqlLiteral) ?
                """, arguments: [value])
            return Set(rows.map { $0["id"] as Int64 })
        }
    }
}
