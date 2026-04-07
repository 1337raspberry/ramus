import Foundation
import os
import PlexAPI

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "SyncEngine")

/// Syncs Plex music library data into the local SQLite cache.
/// `fullSync` fetches all artists/albums/tracks plus deep genre data.
/// `incrementalSync` compares timestamps and only deep-fetches changed albums.
public final class SyncEngine: Sendable {

    public let cache: CacheDatabase
    public let client: PlexClient

    private static let batchSize = 500
    private static let deepGenreConcurrency = 8

    public init(cache: CacheDatabase, client: PlexClient) {
        self.cache = cache
        self.client = client
    }

    // MARK: - Progress

    public struct SyncProgress: Sendable {
        public let phase: Phase
        public let current: Int
        public let total: Int
        public let detail: String

        public var fraction: Double {
            total > 0 ? Double(current) / Double(total) : 0
        }

        public enum Phase: Sendable {
            case artists
            case albums
            case tracks
            case deepGenres
            case done
        }
    }

    // MARK: - Full Sync

    /// Run a full sync: artists, albums, tracks, then deep genre fetch for ALL albums.
    public func fullSync(
        libraryKey: String,
        onProgress: @escaping @Sendable (SyncProgress) -> Void = { _ in }
    ) async throws {
        log.info("full sync starting")
        let start = ContinuousClock.now
        let (albumMap, _) = try await coreSync(libraryKey: libraryKey, incremental: false, onProgress: onProgress)

        // Phase 4: Deep genre sync for ALL albums
        log.info("deep genre sync: fetching all \(albumMap.count) albums")
        try await deepGenreSync(albumMap: albumMap, onlySourceIds: nil, onProgress: onProgress)

        let elapsed = ContinuousClock.now - start
        log.info("full sync complete in \(elapsed)")
        UserDefaults.standard.set(Date().timeIntervalSince1970, forKey: "com.raspsoft.ramus.lastSyncTime")
        onProgress(SyncProgress(phase: .done, current: 1, total: 1, detail: "Sync complete"))
    }

    // MARK: - Genre Sync

    /// Deep genre sync only: re-fetches full metadata for ALL albums to get complete genre lists.
    /// Skips artist/album/track sync — use when you've edited genres on the server.
    public func genreSync(
        libraryKey: String,
        onProgress: @escaping @Sendable (SyncProgress) -> Void = { _ in }
    ) async throws {
        log.info("genre sync starting")
        let start = ContinuousClock.now

        // We need the album map (sourceId → local DB id) without re-syncing everything.
        // Load it from the existing cache.
        let cached = try cache.allAlbumTimestamps()
        let albumMap = Dictionary(uniqueKeysWithValues: cached.map { ($0.key, $0.value.id) })
        log.info("genre sync: fetching all \(albumMap.count) albums")

        try await deepGenreSync(albumMap: albumMap, onlySourceIds: nil, onProgress: onProgress)

        let elapsed = ContinuousClock.now - start
        log.info("genre sync complete in \(elapsed)")
        onProgress(SyncProgress(phase: .done, current: 1, total: 1, detail: "Sync complete"))
    }

    // MARK: - Incremental Sync

    /// Incremental sync: fetches all items but only upserts changed ones and deep-fetches changed/new albums.
    public func incrementalSync(
        libraryKey: String,
        onProgress: @escaping @Sendable (SyncProgress) -> Void = { _ in }
    ) async throws {
        log.info("incremental sync starting")
        let start = ContinuousClock.now
        let (albumMap, changedSourceIds) = try await coreSync(libraryKey: libraryKey, incremental: true, onProgress: onProgress)

        // Phase 4: Deep genre sync only for changed albums
        if !changedSourceIds.isEmpty {
            log.info("deep genre sync: \(changedSourceIds.count) changed albums out of \(albumMap.count)")
            try await deepGenreSync(albumMap: albumMap, onlySourceIds: changedSourceIds, onProgress: onProgress)
        } else {
            log.info("deep genre sync: skipped — 0 albums changed")
        }

        let elapsed = ContinuousClock.now - start
        log.info("incremental sync complete in \(elapsed)")
        UserDefaults.standard.set(Date().timeIntervalSince1970, forKey: "com.raspsoft.ramus.lastSyncTime")
        onProgress(SyncProgress(phase: .done, current: 1, total: 1, detail: "Sync complete"))
    }

    // MARK: - Core Sync (Phases 1-3)

    /// Shared phases 1-3 for both full and incremental sync.
    /// When `incremental` is true, skips upserting items whose `updatedAt` hasn't changed.
    /// Returns the album map and set of changed album sourceIds.
    private func coreSync(
        libraryKey: String,
        incremental: Bool,
        onProgress: @Sendable (SyncProgress) -> Void
    ) async throws -> (albumMap: [String: Int64], changedSourceIds: Set<String>) {
        // Pre-load cached timestamps for incremental mode
        let cachedArtists: [String: CacheDatabase.CachedItemInfo]
        let cachedAlbums: [String: CacheDatabase.CachedAlbumInfo]
        let cachedTracks: [String: CacheDatabase.CachedItemInfo]

        if incremental {
            cachedArtists = (try? cache.allArtistTimestamps()) ?? [:]
            cachedAlbums = (try? cache.allAlbumTimestamps()) ?? [:]
            cachedTracks = (try? cache.allTrackTimestamps()) ?? [:]
        } else {
            cachedArtists = [:]
            cachedAlbums = [:]
            cachedTracks = [:]
        }

        // Phase 1: Artists
        onProgress(SyncProgress(phase: .artists, current: 0, total: 0, detail: "Fetching artists..."))
        let artistItems = try await client.fetchAllItems(libraryKey: libraryKey, type: 8)
        let artistMap = try syncArtists(artistItems, cached: cachedArtists, incremental: incremental, onProgress: onProgress)

        // Phase 2: Albums (returns which albums changed for deep genre sync)
        onProgress(SyncProgress(phase: .albums, current: 0, total: 0, detail: "Fetching albums..."))
        let albumItems = try await client.fetchAllItems(libraryKey: libraryKey, type: 9)
        let (albumMap, changedSourceIds) = try syncAlbums(albumItems, artistMap: artistMap, cached: cachedAlbums, incremental: incremental, onProgress: onProgress)

        // Phase 3: Tracks
        onProgress(SyncProgress(phase: .tracks, current: 0, total: 0, detail: "Fetching tracks..."))
        let trackItems = try await client.fetchAllItems(libraryKey: libraryKey, type: 10)
        try syncTracks(trackItems, albumMap: albumMap, artistMap: artistMap, cached: cachedTracks, incremental: incremental, onProgress: onProgress)

        return (albumMap, changedSourceIds)
    }

    // MARK: - Artist Sync

    /// Returns a map of Plex ratingKey -> local DB id.
    private func syncArtists(
        _ items: [MediaItem],
        cached: [String: CacheDatabase.CachedItemInfo],
        incremental: Bool,
        onProgress: (SyncProgress) -> Void
    ) throws -> [String: Int64] {
        // Pre-seed map from cache (for incremental: all items get an ID, even skipped ones)
        var map: [String: Int64] = [:]
        if incremental {
            for (sourceId, info) in cached {
                map[sourceId] = info.id
            }
        }

        // Partition into changed items
        typealias ArtistTuple = (name: String, sortName: String?, sourceId: String,
                                 artUrl: String?, summary: String?, updatedAt: Int?)
        var changed: [ArtistTuple] = []

        for item in items {
            if incremental, let info = cached[item.ratingKey], info.updatedAt == item.updatedAt {
                continue // unchanged — already in map from cache
            }
            changed.append((name: item.title, sortName: item.titleSort, sourceId: item.ratingKey,
                            artUrl: item.thumb, summary: item.summary, updatedAt: item.updatedAt))
        }

        // Batch upsert changed items
        let total = changed.count
        log.info("artists: \(items.count) fetched, \(total) changed, \(items.count - total) skipped")
        for start in stride(from: 0, to: total, by: Self.batchSize) {
            let end = min(start + Self.batchSize, total)
            let chunk = Array(changed[start..<end])
            let ids = try cache.batchUpsertArtists(chunk)
            map.merge(ids) { _, new in new }
            onProgress(SyncProgress(phase: .artists, current: end, total: total,
                                    detail: "Artists: \(end)/\(total)"))
        }

        if total == 0 {
            onProgress(SyncProgress(phase: .artists, current: items.count, total: items.count,
                                    detail: "Artists: \(items.count) unchanged"))
        }

        return map
    }

    // MARK: - Album Sync

    /// Returns album map AND set of sourceIds for albums that are new or changed.
    private func syncAlbums(
        _ items: [MediaItem],
        artistMap: [String: Int64],
        cached: [String: CacheDatabase.CachedAlbumInfo],
        incremental: Bool,
        onProgress: (SyncProgress) -> Void
    ) throws -> (map: [String: Int64], changedSourceIds: Set<String>) {
        // Pre-seed map from cache
        var map: [String: Int64] = [:]
        if incremental {
            for (sourceId, info) in cached {
                map[sourceId] = info.id
            }
        }

        var changedIds: Set<String> = []

        typealias AlbumTuple = (title: String, artistId: Int64, year: Int?,
                                sourceId: String, artUrl: String?, rating: Double?,
                                studio: String?, updatedAt: Int?,
                                addedAt: Int?, lastViewedAt: Int?)
        var changed: [AlbumTuple] = []
        var genreLinks: [(albumId: Int64, genreName: String)] = []

        for item in items {
            let artistKey = item.parentRatingKey ?? ""
            guard let artistId = artistMap[artistKey] ?? (try? cache.artistId(forSourceId: artistKey)) else {
                continue
            }

            let isChanged: Bool
            if incremental, let info = cached[item.ratingKey] {
                let timestampChanged = info.updatedAt != item.updatedAt
                // Plex doesn't always bump updatedAt for genre-only edits,
                // so also compare the first genre from the list response
                let apiGenre = item.genre?.first?.tag
                let genreChanged = apiGenre != nil && info.firstGenre?.lowercased() != apiGenre?.lowercased()
                isChanged = timestampChanged || genreChanged
                if isChanged {
                    let reason = timestampChanged ? "updatedAt \(info.updatedAt ?? 0)→\(item.updatedAt ?? 0)" :
                                                    "genre '\(info.firstGenre ?? "nil")'→'\(apiGenre ?? "nil")'"
                    log.debug("album changed [\(item.ratingKey)]: \(reason) — \(item.title)")
                }
            } else {
                isChanged = true // new or full sync
            }

            if isChanged {
                changed.append((title: item.title, artistId: artistId, year: item.year,
                                sourceId: item.ratingKey, artUrl: item.thumb,
                                rating: item.userRating, studio: item.studio,
                                updatedAt: item.updatedAt, addedAt: item.addedAt,
                                lastViewedAt: item.lastViewedAt))
                changedIds.insert(item.ratingKey)
            }
        }

        // Batch upsert changed albums
        let total = changed.count
        log.info("albums: \(items.count) fetched, \(total) changed, \(items.count - total) skipped")
        for start in stride(from: 0, to: total, by: Self.batchSize) {
            let end = min(start + Self.batchSize, total)
            let chunk = Array(changed[start..<end])
            let ids = try cache.batchUpsertAlbums(chunk)
            map.merge(ids) { _, new in new }

            // Collect genre links for this chunk
            for albumTuple in chunk {
                if let albumId = ids[albumTuple.sourceId],
                   let item = items.first(where: { $0.ratingKey == albumTuple.sourceId }),
                   let genre = item.genre?.first {
                    genreLinks.append((albumId: albumId, genreName: genre.tag))
                }
            }

            onProgress(SyncProgress(phase: .albums, current: end, total: total,
                                    detail: "Albums: \(end)/\(total)"))
        }

        // Batch upsert genre links
        if !genreLinks.isEmpty {
            try cache.batchUpsertGenresAndLinks(genreLinks)
        }

        if total == 0 {
            onProgress(SyncProgress(phase: .albums, current: items.count, total: items.count,
                                    detail: "Albums: \(items.count) unchanged"))
        }

        return (map, changedIds)
    }

    // MARK: - Track Sync

    private func syncTracks(
        _ items: [MediaItem],
        albumMap: [String: Int64],
        artistMap: [String: Int64],
        cached: [String: CacheDatabase.CachedItemInfo],
        incremental: Bool,
        onProgress: (SyncProgress) -> Void
    ) throws {
        typealias TrackTuple = (title: String, albumId: Int64, artistId: Int64,
                                trackNumber: Int?, discNumber: Int?, durationMs: Int?,
                                sourceId: String, codec: String?, partKey: String?,
                                streamId: Int?, userRating: Double?, bitrate: Int?,
                                trackArtist: String?, updatedAt: Int?)
        var changed: [TrackTuple] = []

        for item in items {
            if incremental, let info = cached[item.ratingKey], info.updatedAt == item.updatedAt {
                continue
            }

            let albumKey = item.parentRatingKey ?? ""
            let artistKey = item.grandparentRatingKey ?? ""

            guard let albumId = albumMap[albumKey] ?? (try? cache.albumId(forSourceId: albumKey)) else {
                continue
            }
            guard let artistId = artistMap[artistKey] ?? (try? cache.artistId(forSourceId: artistKey)) else {
                continue
            }

            let audioStream = item.media?.first?.parts?.first?.streams?.first(where: { $0.streamType == 2 })
            let codec = audioStream?.codec ?? item.media?.first?.audioCodec
            let bitrate = audioStream?.bitrate ?? item.media?.first?.bitrate

            changed.append((title: item.title, albumId: albumId, artistId: artistId,
                            trackNumber: item.index, discNumber: item.parentIndex,
                            durationMs: item.duration, sourceId: item.ratingKey,
                            codec: codec, partKey: item.media?.first?.parts?.first?.key,
                            streamId: audioStream?.id, userRating: item.userRating,
                            bitrate: bitrate, trackArtist: item.originalTitle,
                            updatedAt: item.updatedAt))
        }

        // Batch upsert changed tracks
        let total = changed.count
        log.info("tracks: \(items.count) fetched, \(total) changed, \(items.count - total) skipped")
        for start in stride(from: 0, to: total, by: Self.batchSize) {
            let end = min(start + Self.batchSize, total)
            let chunk = Array(changed[start..<end])
            try cache.batchUpsertTracks(chunk)
            onProgress(SyncProgress(phase: .tracks, current: end, total: total,
                                    detail: "Tracks: \(end)/\(total)"))
        }

        if total == 0 {
            onProgress(SyncProgress(phase: .tracks, current: items.count, total: items.count,
                                    detail: "Tracks: \(items.count) unchanged"))
        }
    }

    // MARK: - Deep Genre Sync

    /// Fetch full metadata for albums to get ALL genres.
    /// Pass `onlySourceIds` to limit which albums are fetched (nil = all).
    /// Uses bounded concurrency (8 requests in flight) for throughput.
    private func deepGenreSync(
        albumMap: [String: Int64],
        onlySourceIds: Set<String>?,
        onProgress: @escaping @Sendable (SyncProgress) -> Void
    ) async throws {
        let albumEntries: [(String, Int64)]
        if let filter = onlySourceIds {
            albumEntries = albumMap.filter { filter.contains($0.key) }.map { ($0.key, $0.value) }
        } else {
            albumEntries = Array(albumMap)
        }

        let total = albumEntries.count
        guard total > 0 else { return }

        let completed = OSAllocatedUnfairLock(initialState: 0)
        let maxConcurrency = Self.deepGenreConcurrency

        try await withThrowingTaskGroup(of: Void.self) { group in
            var inflight = 0

            for (sourceId, albumId) in albumEntries {
                // Bounded concurrency: wait for one to finish before adding more
                if inflight >= maxConcurrency {
                    try await group.next()
                    inflight -= 1
                }

                group.addTask { [cache, client] in
                    do {
                        try await Self.processAlbumDeepSync(
                            sourceId: sourceId, albumId: albumId,
                            client: client, cache: cache
                        )
                    } catch PlexClientError.unauthorized {
                        throw PlexClientError.unauthorized
                    } catch PlexClientError.notConnected {
                        throw PlexClientError.notConnected
                    } catch {
                        // Skip individual album failures (e.g. 404 for deleted albums)
                        return
                    }

                    let count = completed.withLock { state -> Int in
                        state += 1
                        return state
                    }
                    if count % 50 == 0 || count == total {
                        onProgress(SyncProgress(phase: .deepGenres, current: count, total: total,
                                                detail: "Genre sync: \(count)/\(total)"))
                    }
                }
                inflight += 1
            }

            try await group.waitForAll()
        }
    }

    /// Process a single album's deep metadata fetch + DB write.
    private static func processAlbumDeepSync(
        sourceId: String, albumId: Int64,
        client: PlexClient, cache: CacheDatabase
    ) async throws {
        let metadata = try await client.fetchItemMetadata(ratingKey: sourceId)
        let genres = metadata.genre ?? []

        var genreIds: [Int64] = []
        for g in genres {
            let genreId = try cache.upsertGenre(name: g.tag)
            genreIds.append(genreId)
        }

        var colorsString: String?
        if let colors = metadata.ultraBlurColors {
            let colorsJSON = try? JSONEncoder().encode(colors)
            colorsString = colorsJSON.flatMap { String(data: $0, encoding: .utf8) }
        }

        try cache.updateAlbumDeepMetadata(
            albumId: albumId, genreIds: genreIds,
            rating: metadata.userRating, studio: metadata.studio,
            colorsJSON: colorsString
        )
    }
}
