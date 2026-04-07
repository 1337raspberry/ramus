import Foundation
@preconcurrency import Fuse
import Cache
import GenreTree

// MARK: - Types

public enum SearchResultKind: Sendable, Hashable {
    case album
    case track
}

public struct SearchResult: Sendable, Hashable, Identifiable {
    public let id: String
    public let kind: SearchResultKind
    public let albumSourceId: String
    public let albumTitle: String
    public let artistName: String
    public let year: Int?
    public let albumArtPath: String?
    public let trackSourceId: String?
    public let trackTitle: String?
    public let trackArtist: String?
    public let score: Double
}

// MARK: - Engine

public final class SearchEngine: Sendable {

    private let db: CacheDatabase
    private let genreMapper: GenreMapper?
    private let fuse: Fuse

    public init(db: CacheDatabase, genreMapper: GenreMapper? = nil) {
        self.db = db
        self.genreMapper = genreMapper
        self.fuse = Fuse(threshold: 0.4)
    }

    /// Execute a parsed query. Returns albums first, then tracks.
    /// Free-text queries return top 5 albums + top 10 tracks.
    /// Explicit `!` operator returns tracks as before.
    public func search(_ query: ParsedQuery, limit: Int = 100) throws -> [SearchResult] {
        guard !query.isEmpty else { return [] }

        // Resolve album ID constraints from genre + year filters
        let albumIds = try resolveAlbumConstraints(query)
        if let ids = albumIds, ids.isEmpty { return [] }

        // If ONLY track searches (no album-level filters/text), skip albums entirely
        let hasAlbumSearches = query.freeText != nil || !query.artistFilters.isEmpty
            || !query.albumTitleFilters.isEmpty || !query.genreFilters.isEmpty
            || !query.rangeFilters.isEmpty || query.hasFavouritesFilter

        var results: [SearchResult] = []

        if hasAlbumSearches {
            let albumLimit = query.isFreeTextOnly ? 5 : limit
            let albumResults = try searchAlbums(query, albumIds: albumIds, limit: albumLimit)
            results.append(contentsOf: albumResults)
        }

        // Explicit ! operator track search
        if query.hasTrackSearch {
            let trackResults = try searchTracks(query, albumIds: albumIds, limit: limit)
            results.append(contentsOf: trackResults)
        }

        // Supplementary track search for free text (no ! operator)
        if let text = query.freeText, !query.hasTrackSearch {
            let trackResults = try searchTracksByText(text, albumIds: albumIds, limit: 10)
            results.append(contentsOf: trackResults)
        }

        return results
    }

    // MARK: - Album Search

    private func searchAlbums(_ query: ParsedQuery, albumIds: Set<Int64>?, limit: Int) throws -> [SearchResult] {
        var seen = Set<String>()
        var results: [SearchResult] = []

        // % operator: search by album title specifically
        for titleQuery in query.albumTitleFilters {
            let rows = try db.searchAlbumsByTitle(query: titleQuery, albumIds: albumIds, limit: limit)
            for row in rows where !seen.contains(row.albumSourceId) {
                seen.insert(row.albumSourceId)
                let score = matchScore(row.albumTitle, query: titleQuery)
                results.append(SearchResult(
                    id: "album-\(row.albumSourceId)", kind: .album,
                    albumSourceId: row.albumSourceId, albumTitle: row.albumTitle,
                    artistName: row.artistName, year: row.year, albumArtPath: row.artUrl,
                    trackSourceId: nil, trackTitle: nil, trackArtist: nil,
                    score: score
                ))
            }
        }

        // @ operator: search by artist name
        for artistQuery in query.artistFilters {
            let rows = try db.searchAlbumsByArtist(query: artistQuery, albumIds: albumIds, limit: limit)
            for row in rows where !seen.contains(row.albumSourceId) {
                seen.insert(row.albumSourceId)
                let score = matchScore(row.artistName, query: artistQuery)
                results.append(SearchResult(
                    id: "album-\(row.albumSourceId)", kind: .album,
                    albumSourceId: row.albumSourceId, albumTitle: row.albumTitle,
                    artistName: row.artistName, year: row.year, albumArtPath: row.artUrl,
                    trackSourceId: nil, trackTitle: nil, trackArtist: nil,
                    score: score
                ))
            }
        }

        // Free text: search both artist name and album title in one query
        if let text = query.freeText {
            let rows = try db.searchAlbumsByArtistOrTitle(query: text, albumIds: albumIds, limit: limit)
            for row in rows where !seen.contains(row.albumSourceId) {
                seen.insert(row.albumSourceId)
                let artistScore = matchScore(row.artistName, query: text)
                let titleScore = matchScore(row.albumTitle, query: text)
                let score = min(artistScore, titleScore) + 0.1
                results.append(SearchResult(
                    id: "album-\(row.albumSourceId)", kind: .album,
                    albumSourceId: row.albumSourceId, albumTitle: row.albumTitle,
                    artistName: row.artistName, year: row.year, albumArtPath: row.artUrl,
                    trackSourceId: nil, trackTitle: nil, trackArtist: nil,
                    score: score
                ))
            }
        }

        // Filters only (genre/year with no text) — list matching albums
        if query.freeText == nil && query.artistFilters.isEmpty
            && query.albumTitleFilters.isEmpty && albumIds != nil {
            let rows = try db.searchAlbums(albumIds: albumIds, limit: limit, randomOrder: true)
            for row in rows where !seen.contains(row.albumSourceId) {
                seen.insert(row.albumSourceId)
                results.append(SearchResult(
                    id: "album-\(row.albumSourceId)", kind: .album,
                    albumSourceId: row.albumSourceId, albumTitle: row.albumTitle,
                    artistName: row.artistName, year: row.year, albumArtPath: row.artUrl,
                    trackSourceId: nil, trackTitle: nil, trackArtist: nil,
                    score: 0.0
                ))
            }
        }

        return Array(results.sorted { $0.score < $1.score }.prefix(limit))
    }

    // MARK: - Track Search

    private func searchTracks(_ query: ParsedQuery, albumIds: Set<Int64>?, limit: Int) throws -> [SearchResult] {
        var results: [SearchResult] = []

        for trackQuery in query.trackSearches {
            let partial = try searchTracksByText(trackQuery, albumIds: albumIds, limit: limit)
            results.append(contentsOf: partial)
        }

        return Array(results.sorted { $0.score < $1.score }.prefix(limit))
    }

    /// Search tracks by a single text string using FTS5 prefix matching with Fuse fuzzy fallback.
    private func searchTracksByText(_ text: String, albumIds: Set<Int64>?, limit: Int) throws -> [SearchResult] {
        var seen = Set<Int64>()
        var results: [SearchResult] = []

        let escaped = QueryParser.escapeFTS5(text)
        let ftsTokens = escaped
            .components(separatedBy: .whitespaces)
            .filter { !$0.isEmpty }
            .map { "\"\($0)\"*" }
            .joined(separator: " ")

        if !ftsTokens.isEmpty {
            let ftsResults = try db.searchTracksEnriched(
                ftsQuery: ftsTokens, albumIds: albumIds, limit: limit
            )
            for row in ftsResults where !seen.contains(row.id) {
                seen.insert(row.id)
                let score = matchScore(row.trackTitle, query: text)
                results.append(SearchResult(
                    id: "track-\(row.id)", kind: .track,
                    albumSourceId: row.albumSourceId, albumTitle: row.albumTitle,
                    artistName: row.artistName, year: nil, albumArtPath: row.artUrl,
                    trackSourceId: row.trackSourceId, trackTitle: row.trackTitle,
                    trackArtist: row.trackArtist,
                    score: score
                ))
            }
        }

        // Fuse fallback if < 5 track results
        if results.count < 5 {
            let fuzzyResults = try fuzzyTrackSearch(text, albumIds: albumIds, excluding: seen)
            results.append(contentsOf: fuzzyResults)
        }

        return Array(results.sorted { $0.score < $1.score }.prefix(limit))
    }

    // MARK: - Private

    private func resolveAlbumConstraints(_ query: ParsedQuery) throws -> Set<Int64>? {
        var constrainedIds: Set<Int64>?

        let genres = query.genreFilters
        if !genres.isEmpty {
            // Expand each genre to include all descendants in the genre hierarchy
            var expandedNames = Set<String>()
            for genre in genres {
                if let mapper = genreMapper, let node = mapper.matchGenre(genre) {
                    node.collectDescendantNames(into: &expandedNames)
                } else {
                    expandedNames.insert(genre)
                }
            }
            let genreAlbumIds = try db.albumIdsForGenreNames(Array(expandedNames))
            constrainedIds = genreAlbumIds
        }

        for (field, op, value) in query.rangeFilters {
            let matchedIds: Set<Int64>
            switch field {
            case .year:
                matchedIds = try db.albumIdsForYearRange(op: op, value: Int(value))
            case .rating:
                matchedIds = try db.albumIdsForRatingRange(op: op, value: value)
            }
            if let existing = constrainedIds {
                constrainedIds = existing.intersection(matchedIds)
            } else {
                constrainedIds = matchedIds
            }
        }

        if query.hasFavouritesFilter {
            let favIds = try db.albumIdsForFavourites()
            if let existing = constrainedIds {
                constrainedIds = existing.intersection(favIds)
            } else {
                constrainedIds = favIds
            }
        }

        return constrainedIds
    }

    private func fuzzyTrackSearch(
        _ text: String, albumIds: Set<Int64>?, excluding: Set<Int64>
    ) throws -> [SearchResult] {
        let candidates = try db.searchCandidates(albumIds: albumIds, limit: 5000)
        guard let pattern = fuse.createPattern(from: text) else { return [] }

        var scored: [(id: Int64, trackSourceId: String, trackTitle: String, artistName: String, albumTitle: String, albumSourceId: String, artUrl: String?, trackArtist: String?, score: Double)] = []

        for candidate in candidates where !excluding.contains(candidate.id) {
            let composite = "\(candidate.artistName) \(candidate.albumTitle) \(candidate.trackTitle)"
            if let result = fuse.search(pattern, in: composite) {
                scored.append((
                    candidate.id, candidate.trackSourceId, candidate.trackTitle,
                    candidate.artistName, candidate.albumTitle, candidate.albumSourceId,
                    candidate.artUrl, candidate.trackArtist, result.score
                ))
            }
        }

        scored.sort { $0.score < $1.score }

        return scored.prefix(50).map { item in
            SearchResult(
                id: "track-\(item.id)", kind: .track,
                albumSourceId: item.albumSourceId, albumTitle: item.albumTitle,
                artistName: item.artistName, year: nil, albumArtPath: item.artUrl,
                trackSourceId: item.trackSourceId, trackTitle: item.trackTitle,
                trackArtist: item.trackArtist,
                score: 0.5 + item.score
            )
        }
    }

    /// Score a match: exact = 0.0, starts-with = 0.02, contains = 0.05
    private func matchScore(_ value: String, query: String) -> Double {
        let v = value.lowercased()
        let q = query.lowercased()
        if v == q { return 0.0 }
        if v.hasPrefix(q) { return 0.02 }
        return 0.05
    }

}
