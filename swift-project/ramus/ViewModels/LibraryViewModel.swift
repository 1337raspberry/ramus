import SwiftUI
import Cache
import Models
import GenreTree
import PlexAPI
import Search
import os

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "LibraryViewModel")

// MARK: - Sidebar Mode

enum SidebarMode: String, CaseIterable, Identifiable {
    case genres, favourites, artists
    var id: String { rawValue }
    var label: String {
        switch self {
        case .genres: "Genres"
        case .favourites: "★ Starred"
        case .artists: "Artists"
        }
    }
}

// MARK: - Album Sort Order

enum AlbumSortOrder: String, CaseIterable, Identifiable {
    case alphabetical, latestAdded, recentlyPlayed, random
    var id: String { rawValue }
    var label: String {
        switch self {
        case .alphabetical: "Alphabetical"
        case .latestAdded: "Latest Added"
        case .recentlyPlayed: "Most Recently Played"
        case .random: "Random"
        }
    }
    var icon: String {
        switch self {
        case .alphabetical: "textformat.abc"
        case .latestAdded: "calendar.badge.plus"
        case .recentlyPlayed: "memories"
        case .random: "shuffle"
        }
    }
}

// MARK: - Genre Source

enum GenreSource: String, CaseIterable, Identifiable {
    case open, custom
    var id: String { rawValue }
    var label: String {
        switch self {
        case .open: "Wikidata (CC0)"
        case .custom: "Custom"
        }
    }
    var resourceName: String {
        switch self {
        case .open: "open"
        case .custom: "" // not used — .custom loads from container path
        }
    }
}

/// Drives genre/album/track navigation in the three-column layout.
@MainActor @Observable
final class LibraryViewModel {

    // MARK: - Dependencies

    var cache: CacheDatabase?
    var genreMapper: GenreMapper?
    var player: PlaybackViewModel?

    // MARK: - Sidebar

    var sidebarMode: SidebarMode = .genres
    var albumSortOrder: AlbumSortOrder = .random

    // MARK: - Genre Tree

    var genreSource: GenreSource = {
        let raw = UserDefaults.standard.string(forKey: UserDefaultsKeys.genreSource) ?? GenreSource.open.rawValue
        let source = GenreSource(rawValue: raw) ?? .open
        if source == .custom && !FileManager.default.fileExists(atPath: LibraryViewModel.customGenresURL.path) {
            UserDefaults.standard.set(GenreSource.open.rawValue, forKey: UserDefaultsKeys.genreSource)
            return .open
        }
        return source
    }() {
        didSet {
            UserDefaults.standard.set(genreSource.rawValue, forKey: UserDefaultsKeys.genreSource)
            reloadGenreMapper()
        }
    }

    // MARK: - Custom Genre Import

    /// Fixed location for the user's custom genre JSON in the app container.
    static var customGenresURL: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("ramus", isDirectory: true)
            .appendingPathComponent("custom_genres.json")
    }

    var hasCustomGenreFile: Bool {
        FileManager.default.fileExists(atPath: Self.customGenresURL.path)
    }

    var customImportError: String?
    var customImportWarnings: [String] = []
    var customImportFileName: String? = UserDefaults.standard.string(forKey: UserDefaultsKeys.customGenreFileName)

    var useHierarchy: Bool = {
        if let stored = UserDefaults.standard.object(forKey: UserDefaultsKeys.useHierarchy) as? Bool {
            return stored
        }
        return true
    }() {
        didSet { UserDefaults.standard.set(useHierarchy, forKey: UserDefaultsKeys.useHierarchy) }
    }
    var libraryPadding: CGFloat = {
        let val = UserDefaults.standard.object(forKey: UserDefaultsKeys.libraryPadding) as? Double
        return CGFloat(val ?? 8)
    }() {
        didSet {
            libraryPaddingSaveTask?.cancel()
            libraryPaddingSaveTask = Task { @MainActor [padding = libraryPadding] in
                try? await Task.sleep(for: .milliseconds(300))
                UserDefaults.standard.set(Double(padding), forKey: UserDefaultsKeys.libraryPadding)
            }
        }
    }
    private var libraryPaddingSaveTask: Task<Void, Never>?
    var flatGenreList: [(name: String, count: Int)] = []
    var flatFavouriteGenreList: [(name: String, count: Int)] = []
    var genreTree: [GenreNode] = []
    var allExpandableGenreIDs: Set<String> = []
    var selectedGenre: GenreNode?
    var expandedGenreIDs: Set<String> = []
    var scrollToGenreID: String?

    // MARK: - Favourites Tree

    var favouriteGenreTree: [GenreNode] = []
    var allExpandableFavouriteIDs: Set<String> = []
    var favouriteAlbumCount: Int = 0

    // MARK: - Artists

    var artists: [(id: Int64, name: String, sourceId: String, artUrl: String?)] = []
    var selectedArtistSourceId: String?

    // MARK: - Albums (content column)

    var albums: [Album] = []
    var totalAlbumCount: Int = 0
    var selectedAlbum: Album?

    // MARK: - Tracks (for selected album)

    var tracks: [Track] = []

    // MARK: - State

    /// Suppresses `selectGenre` side effects during search-to-grid loads.
    private var suppressGenreSelection = false

    // MARK: - Sentinel

    static func allNode(count: Int = 0) -> GenreNode {
        var node = GenreNode(name: "All", parentPath: "__sentinel__", albumCount: count, deduplicatedTotalCount: count)
        node.children = []
        return node
    }

    // Tracks cycling index per genre name for multi-location navigation
    private var genreNavCycleIndex: [String: Int] = [:]

    // MARK: - Setup

    func setup(cache: CacheDatabase?, player: PlaybackViewModel) {
        self.cache = cache
        self.player = player
        loadGenreMapper()
        refreshGenreTree()
    }

    private func loadGenreMapper() {
        do {
            if genreSource == .custom {
                genreMapper = try GenreMapper(jsonURL: Self.customGenresURL)
            } else {
                genreMapper = try GenreMapper(bundle: .main, resourceName: genreSource.resourceName)
            }
        } catch {
            log.error("Failed to load genre mapper: \(error.localizedDescription, privacy: .public)")
            genreMapper = nil
        }
    }

    /// Incremented on every genre mapper reload so observers can react.
    private(set) var genreMapperVersion: Int = 0

    private func reloadGenreMapper() {
        loadGenreMapper()
        refreshGenreTree()
        genreMapperVersion += 1
        selectedGenre = nil
        albums = []
        selectedAlbum = nil
        tracks = []
        expandedGenreIDs = []
        genreNavCycleIndex = [:]
    }

    /// Import a custom genres text file. Parses, validates, saves to container, switches source.
    func importCustomGenres(text: String, fileName: String) throws {
        let (jsonData, warnings) = try CustomGenreParser.parse(text)

        // Ensure app support directory exists
        let dir = Self.customGenresURL.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try jsonData.write(to: Self.customGenresURL, options: .atomic)

        UserDefaults.standard.set(fileName, forKey: UserDefaultsKeys.customGenreFileName)
        customImportFileName = fileName
        customImportWarnings = warnings
        customImportError = nil
        if genreSource == .custom {
            // Already on .custom — didSet won't fire, so reload manually
            reloadGenreMapper()
        } else {
            genreSource = .custom // triggers didSet → reloadGenreMapper()
        }
    }

    /// Remove the custom genre file and switch back to default source.
    func removeCustomGenres() {
        try? FileManager.default.removeItem(at: Self.customGenresURL)
        UserDefaults.standard.removeObject(forKey: UserDefaultsKeys.customGenreFileName)
        customImportFileName = nil
        customImportWarnings = []
        customImportError = nil
        if genreSource == .custom {
            genreSource = .open
        }
    }

    // MARK: - Genre Tree

    func refreshGenreTree() {
        let (tree, flat) = buildGenreTree { try $0.genreAlbumSets() }
        genreTree = tree
        flatGenreList = flat
        allExpandableGenreIDs = Self.collectAllExpandableIDs(in: tree)
        totalAlbumCount = (try? cache?.stats())?.albumCount ?? 0
    }

    // MARK: - Favourites Tree

    func refreshFavouriteGenreTree() {
        let (tree, flat) = buildGenreTree { try $0.favouriteAlbumGenreSets() }
        favouriteGenreTree = tree
        flatFavouriteGenreList = flat
        allExpandableFavouriteIDs = Self.collectAllExpandableIDs(in: tree)
        favouriteAlbumCount = (try? cache?.albumIdsForFavourites())?.count ?? 0
    }

    /// Collects every expandable node ID in a genre tree (nodes with non-empty children).
    private static func collectAllExpandableIDs(in nodes: [GenreNode]) -> Set<String> {
        var ids = Set<String>()
        for node in nodes {
            if let children = node.children, !children.isEmpty {
                ids.insert(node.id)
                ids.formUnion(collectAllExpandableIDs(in: children))
            }
        }
        return ids
    }

    /// Shared logic for building a genre tree + flat list from a cache query.
    private func buildGenreTree(
        fetchSets: (CacheDatabase) throws -> [String: Set<Int64>]
    ) -> (tree: [GenreNode], flat: [(name: String, count: Int)]) {
        guard let cache, let genreMapper else { return ([], []) }
        do {
            let albumSets = try fetchSets(cache)
            let tree = genreMapper.buildDisplayTree(genreAlbumSets: albumSets)
            let flat = albumSets.map { (name: $0.key, count: $0.value.count) }
                .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
            return (tree, flat)
        } catch {
            log.error("Failed to load genres: \(error.localizedDescription, privacy: .public)")
            return ([], [])
        }
    }

    // MARK: - Artists

    func loadAllArtists() {
        guard let cache else { return }
        do {
            artists = try cache.allArtists()
        } catch {
            log.error("Failed to load artists: \(error.localizedDescription, privacy: .public)")
        }
    }

    func selectArtist(sourceId: String) {
        selectedArtistSourceId = sourceId
        selectedGenre = nil
        selectedAlbum = nil
        tracks = []
        guard let cache else { return }
        do {
            let rows = try cache.albumsByArtistSourceId(sourceId)
            albums = Self.albumsFromRows(rows)
            sortAlbums()
        } catch {
            log.error("Failed to load artist albums: \(error.localizedDescription, privacy: .public)")
            albums = []
        }
    }

    // MARK: - All Albums

    func loadAllAlbums() {
        guard let cache else { return }
        selectedAlbum = nil
        tracks = []
        do {
            let rows = try cache.allAlbums()
            albums = Self.albumsFromRows(rows)
            sortAlbums()
        } catch {
            log.error("Failed to load albums: \(error.localizedDescription, privacy: .public)")
            albums = []
        }
    }

    func loadAllFavouriteAlbums() {
        guard let cache else { return }
        selectedAlbum = nil
        tracks = []
        do {
            let rows = try cache.allFavouriteAlbums()
            albums = Self.albumsFromRows(rows)
            sortAlbums()
        } catch {
            log.error("Failed to load favourite albums: \(error.localizedDescription, privacy: .public)")
            albums = []
        }
    }

    // MARK: - Sorting

    func sortAlbums() {
        switch albumSortOrder {
        case .alphabetical:
            albums.sort {
                let cmp = $0.artistName.localizedCaseInsensitiveCompare($1.artistName)
                if cmp != .orderedSame { return cmp == .orderedAscending }
                return ($0.year ?? 0) < ($1.year ?? 0)
            }
        case .latestAdded:
            albums.sort { ($0.addedAt ?? 0) > ($1.addedAt ?? 0) }
        case .recentlyPlayed:
            albums.sort { ($0.lastViewedAt ?? 0) > ($1.lastViewedAt ?? 0) }
        case .random:
            albums.shuffle()
        }
    }

    // MARK: - Row → Album helper

    /// Convert an album row tuple into an Album model.
    /// Works for both 10-element (list queries) and 11-element (single-album queries with artistSourceId).
    static func albumFromRow(_ row: (id: Int64, title: String, artistName: String, year: Int?, artUrl: String?, sourceId: String, rating: Double?, studio: String?, addedAt: Int?, lastViewedAt: Int?)) -> Album {
        Album(
            ratingKey: row.sourceId,
            title: row.title,
            artistName: row.artistName,
            year: row.year,
            thumb: row.artUrl,
            genres: [],
            isFavourite: (row.rating ?? 0) >= 10.0,
            studio: row.studio,
            addedAt: row.addedAt,
            lastViewedAt: row.lastViewedAt
        )
    }

    /// Overload for 11-element tuples (albumForSourceId queries that include artistSourceId).
    static func albumFromRow(_ row: (id: Int64, title: String, artistName: String, year: Int?, artUrl: String?, sourceId: String, rating: Double?, studio: String?, addedAt: Int?, lastViewedAt: Int?, artistSourceId: String?)) -> Album {
        albumFromRow((row.id, row.title, row.artistName, row.year, row.artUrl, row.sourceId, row.rating, row.studio, row.addedAt, row.lastViewedAt))
    }

    private static func albumsFromRows(_ rows: [(id: Int64, title: String, artistName: String, year: Int?, artUrl: String?, sourceId: String, rating: Double?, studio: String?, addedAt: Int?, lastViewedAt: Int?)]) -> [Album] {
        rows.map { albumFromRow($0) }
    }

    /// Select a flat genre by name — creates a leaf GenreNode and delegates to selectGenre.
    func selectFlatGenre(named name: String) {
        let node = GenreNode(name: name, children: [])
        selectGenre(node)
    }

    // MARK: - Genre Selection → Albums

    func selectGenre(_ genre: GenreNode?) {
        guard !suppressGenreSelection else { return }
        selectedGenre = genre
        selectedArtistSourceId = nil
        selectedAlbum = nil
        tracks = []

        guard let genre, let cache else {
            albums = []
            return
        }

        // Handle "All" sentinel
        if genre.id == Self.allNode().id {
            if sidebarMode == .favourites {
                loadAllFavouriteAlbums()
            } else {
                loadAllAlbums()
            }
            return
        }

        do {
            let descendantNames = genre.allDescendantNames
            let genreIds = try cache.genreIds(forNames: descendantNames)
            var rows = try cache.albumsByGenreIds(genreIds)
            if sidebarMode == .favourites {
                rows = rows.filter { ($0.rating ?? 0) >= 10 }
            }
            albums = Self.albumsFromRows(rows)
            sortAlbums()
        } catch {
            log.error("Failed to load genre albums: \(error.localizedDescription, privacy: .public)")
            albums = []
        }
    }

    // MARK: - Album Selection → Tracks

    func fetchTracks(for album: Album) -> [Track] {
        guard let cache else { return [] }
        do {
            let rows = try cache.tracksForAlbum(sourceId: album.ratingKey)
            return rows.map { row in
                Track(
                    ratingKey: row.sourceId,
                    title: row.title,
                    artistName: album.artistName,
                    trackArtist: row.trackArtist,
                    albumTitle: album.title,
                    albumKey: album.ratingKey,
                    index: row.trackNumber,
                    duration: Double(row.durationMs ?? 0) / 1000.0,
                    codec: row.codec,
                    partKey: row.partKey,
                    thumb: album.thumb,
                    isFavourite: (row.userRating ?? 0) >= 10.0,
                    bitrate: row.bitrate,
                    discNumber: row.discNumber
                )
            }
        } catch {
            log.error("Failed to load tracks: \(error.localizedDescription, privacy: .public)")
            return []
        }
    }

    func selectAlbum(_ album: Album?) {
        selectedAlbum = album
        guard let album else {
            tracks = []
            return
        }
        tracks = fetchTracks(for: album)
    }

    func playAlbumInPlace(_ album: Album, startAt index: Int = 0) {
        let albumTracks = fetchTracks(for: album)
        guard !albumTracks.isEmpty else { return }
        player?.playTracks(albumTracks, startAt: index)
        try? cache?.updateAlbumLastViewed(sourceId: album.ratingKey, at: Int(Date().timeIntervalSince1970))
    }

    /// Play an album by sourceId without navigating (for search results).
    func playAlbumBySourceId(_ sourceId: String) {
        guard let cache else { return }
        guard let row = try? cache.albumForSourceId(sourceId) else { return }
        let album = Self.albumFromRow(row)
        playAlbumInPlace(album)
    }

    /// Returns tracks for a search result: all album tracks for albums, single track for track results.
    func tracksForSearchResult(_ result: Search.SearchResult) -> [Track] {
        let albumTracks = fetchTracksByAlbumSourceId(result.albumSourceId)
        if result.kind == .track, let trackId = result.trackSourceId {
            return albumTracks.filter { $0.ratingKey == trackId }
        }
        return albumTracks
    }

    func playTrackFromSearch(albumSourceId: String, trackSourceId: String) {
        let tracks = fetchTracksByAlbumSourceId(albumSourceId)
        guard !tracks.isEmpty else { return }
        let index = tracks.firstIndex(where: { $0.ratingKey == trackSourceId }) ?? 0
        player?.playTracks(tracks, startAt: index)
    }

    func fetchTracksByAlbumSourceId(_ sourceId: String) -> [Track] {
        guard let cache else { return [] }
        guard let row = try? cache.albumForSourceId(sourceId) else { return [] }
        let album = Self.albumFromRow(row)
        return fetchTracks(for: album)
    }

    // MARK: - Playback

    func playTrack(at index: Int) {
        player?.playTracks(tracks, startAt: index)
    }

    // MARK: - Shuffle Favourites

    func shuffleFavouriteTracks() {
        guard let cache, let player else { return }
        do {
            let rows = try cache.allFavouriteTracks()
            guard !rows.isEmpty else { return }
            var tracks = rows.map { row in
                Track(
                    ratingKey: row.sourceId, title: row.title,
                    artistName: row.artistName, trackArtist: row.trackArtist,
                    albumTitle: row.albumTitle,
                    albumKey: row.albumSourceId, index: row.trackNumber,
                    duration: Double(row.durationMs ?? 0) / 1000.0,
                    codec: row.codec, partKey: row.partKey,
                    thumb: row.albumArtUrl,
                    isFavourite: (row.userRating ?? 0) >= 10.0, bitrate: row.bitrate,
                    discNumber: row.discNumber
                )
            }
            tracks.shuffle()
            player.playTracks(tracks)
        } catch {
            // Silent — non-critical
        }
    }

    // MARK: - Favourites

    func toggleFavourite(album: Album) {
        guard let cache, let player else { return }
        let newFavourite = !album.isFavourite
        let newRating: Double = newFavourite ? 10.0 : 0.0

        // Optimistic local update
        try? cache.updateAlbumRating(sourceId: album.ratingKey, rating: newRating)

        if let idx = albums.firstIndex(where: { $0.id == album.id }) {
            albums[idx] = Album(
                ratingKey: album.ratingKey, title: album.title,
                artistName: album.artistName,
                year: album.year, thumb: album.thumb, genres: album.genres,
                isFavourite: newFavourite, studio: album.studio,
                addedAt: album.addedAt, lastViewedAt: album.lastViewedAt
            )
        }

        Task {
            try? await player.plexClient.rateItem(ratingKey: album.ratingKey, rating: newRating)
        }
    }

    func toggleFavourite(track: Track) {
        guard let cache, let player else { return }
        let newFavourite = !track.isFavourite
        let newRating: Double = newFavourite ? 10.0 : 0.0

        try? cache.updateTrackRating(sourceId: track.ratingKey, userRating: newRating)

        if let idx = tracks.firstIndex(where: { $0.id == track.id }) {
            tracks[idx] = Track(
                ratingKey: track.ratingKey, title: track.title,
                artistName: track.artistName, trackArtist: track.trackArtist,
                albumTitle: track.albumTitle,
                albumKey: track.albumKey, index: track.index,
                duration: track.duration, codec: track.codec,
                partKey: track.partKey, thumb: track.thumb,
                isFavourite: newFavourite, bitrate: track.bitrate,
                discNumber: track.discNumber
            )
        }

        Task {
            try? await player.plexClient.rateItem(ratingKey: track.ratingKey, rating: newRating)
        }
    }

    // MARK: - Genre Navigation (from pills)

    /// Navigate to a genre by name. Cycles between multiple occurrences
    /// in the tree on repeated clicks, and expands the path to the target.
    func navigateToGenre(named name: String) {
        guard let genreMapper else { return }
        guard let matched = genreMapper.matchGenre(name) else { return }

        let allMatches = findAllNodes(named: matched.name, in: genreTree)
        guard !allMatches.isEmpty else { return }

        // Cycle through matches on repeated clicks
        let idx = genreNavCycleIndex[matched.name, default: 0] % allMatches.count
        genreNavCycleIndex[matched.name] = idx + 1
        let target = allMatches[idx]

        // Expand the path to this node
        expandPathTo(id: target.id, in: genreTree)

        // Set selection — onChange in SidebarView will call selectGenre
        selectedGenre = target
        scrollToGenreID = target.id
    }

    /// Find ALL nodes with a given name across the entire display tree.
    private func findAllNodes(named name: String, in nodes: [GenreNode]) -> [GenreNode] {
        var results: [GenreNode] = []
        for node in nodes {
            if node.name == name {
                results.append(node)
            }
            if let children = node.children {
                results.append(contentsOf: findAllNodes(named: name, in: children))
            }
        }
        return results
    }

    /// Expand all ancestor nodes on the path to the given node ID.
    @discardableResult
    private func expandPathTo(id targetID: String, in nodes: [GenreNode]) -> Bool {
        for node in nodes {
            if node.id == targetID {
                return true
            }
            if let children = node.children {
                if expandPathTo(id: targetID, in: children) {
                    expandedGenreIDs.insert(node.id)
                    return true
                }
            }
        }
        return false
    }

    /// Suppress genre selection, clear nav state, load albums, then restore.
    /// Used by search, year, and artist "external" loads that bypass genre selection.
    private func loadAlbumsExternally(_ load: () throws -> [Album]) {
        suppressGenreSelection = true
        selectedGenre = nil
        selectedArtistSourceId = nil
        selectedAlbum = nil
        tracks = []

        do {
            albums = try load()
            sortAlbums()
        } catch {
            albums = []
        }

        Task { @MainActor [self] in
            suppressGenreSelection = false
        }
    }

    /// Load search result albums into the grid, clearing sidebar state.
    func loadAlbumsFromSearch(_ results: [SearchResult]) {
        guard let cache else { return }
        loadAlbumsExternally {
            results.compactMap { result in
                guard let row = try? cache.albumForSourceId(result.albumSourceId) else { return nil }
                return Self.albumFromRow(row)
            }
        }
    }

    /// Load all albums from a given year into the grid.
    func loadAlbumsByYear(_ year: Int) {
        guard let cache else { return }
        loadAlbumsExternally {
            let ids = try cache.albumIdsForYearRange(op: .equal, value: year)
            let rows = try cache.searchAlbums(albumIds: ids)
            return rows.map { row in
                Album(
                    ratingKey: row.albumSourceId, title: row.albumTitle,
                    artistName: row.artistName, year: row.year,
                    thumb: row.artUrl, genres: [],
                    isFavourite: (row.rating ?? 0) >= 10.0
                )
            }
        }
    }

    /// Load all albums by a given artist into the grid.
    func loadAlbumsByArtist(sourceId: String) {
        guard let cache else { return }
        loadAlbumsExternally {
            let rows = try cache.albumsByArtistSourceId(sourceId)
            return Self.albumsFromRows(rows)
        }
    }
}
