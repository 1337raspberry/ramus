import Foundation
import os
import Models

private let logger = Logger(subsystem: "com.raspsoft.ramus", category: "PlexClient")

/// HTTP client for Plex server communication via raw URLSession.
public final class PlexClient: Sendable {

    public let clientIdentifier: String

    /// Thread-safe storage for mutable connection state.
    /// URL + token are always read/written as an atomic pair to prevent
    /// a concurrent `get()`/`put()` from reading a new URL with an old token.
    private struct ConnectionState: Sendable {
        var serverURL: URL?
        var token: String?
        var onRequestFailed: (@Sendable () async -> Bool)?
    }

    private let _state = OSAllocatedUnfairLock(initialState: ConnectionState())

    public var serverURL: URL? {
        get { _state.withLock { $0.serverURL } }
        set { _state.withLock { $0.serverURL = newValue } }
    }

    public var token: String? {
        get { _state.withLock { $0.token } }
        set { _state.withLock { $0.token = newValue } }
    }

    /// Called when a request fails with a connection error.
    /// Return `true` if reconnection succeeded and the request should be retried.
    public var onRequestFailed: (@Sendable () async -> Bool)? {
        get { _state.withLock { $0.onRequestFailed } }
        set { _state.withLock { $0.onRequestFailed = newValue } }
    }

    // MARK: - Init

    public init() {
        self.clientIdentifier = Self.persistentClientIdentifier()
    }

    private static func persistentClientIdentifier() -> String {
        let key = "com.raspsoft.ramus.clientIdentifier"
        if let existing = UserDefaults.standard.string(forKey: key) {
            return existing
        }
        let id = UUID().uuidString
        UserDefaults.standard.set(id, forKey: key)
        return id
    }

    // MARK: - Server Discovery (plex.tv Resources API)

    /// Fetch all servers available to the authenticated user via plex.tv.
    public func discoverServers(authToken: String) async throws -> [PlexServer] {
        var components = URLComponents(string: "https://plex.tv/api/v2/resources")!
        components.queryItems = [
            URLQueryItem(name: "includeHttps", value: "1"),
            URLQueryItem(name: "includeRelay", value: "1"),
        ]
        var request = URLRequest(url: components.url!)
        applyStandardHeaders(to: &request, token: authToken)

        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw PlexClientError.connectionFailed
        }
        guard data.count <= Self.maxResponseBytes else {
            throw PlexClientError.invalidResponse
        }

        let resources = try JSONDecoder().decode([PlexResourceResponse].self, from: data)
        return resources
            .filter { $0.provides.contains("server") && $0.accessToken != nil }
            .compactMap { resource in
                guard let token = resource.accessToken else { return nil as PlexServer? }
                return PlexServer(
                    machineIdentifier: resource.clientIdentifier,
                    name: resource.name,
                    accessToken: token,
                    owned: resource.owned ?? false,
                    connections: (resource.connections ?? []).map { conn in
                        PlexServerConnection(
                            uri: conn.uri,
                            local: conn.local ?? false,
                            relay: conn.relay ?? false,
                            protocol: conn.protocol ?? "https"
                        )
                    }
                )
            }
    }

    /// Test a single connection URI with GET /identity.
    public func testConnection(uri: String, token: String, timeout: TimeInterval = 5) async -> Bool {
        guard let base = URL(string: uri),
              let scheme = base.scheme?.lowercased(),
              scheme == "http" || scheme == "https"
        else { return false }
        guard let url = base.appendingPathComponent("identity") as URL? else { return false }
        var request = URLRequest(url: url, timeoutInterval: timeout)
        applyStandardHeaders(to: &request, token: token)
        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            return (response as? HTTPURLResponse)?.statusCode == 200
        } catch {
            return false
        }
    }

    /// Find best working connection for a server. Tests all connections concurrently,
    /// returns the highest-priority one that succeeds. Avoids wasting time on sequential
    /// timeouts (e.g. 5s timeout on unreachable LAN address before trying remote).
    ///
    /// - Parameter allowHTTP: When `false`, HTTP connections are excluded from candidates.
    /// - Returns: The best working connection and whether it uses plaintext HTTP.
    public func findBestConnection(server: PlexServer, allowHTTP: Bool = true) async -> (connection: PlexServerConnection?, isHTTP: Bool) {
        let sorted = allowHTTP
            ? server.sortedConnections
            : server.sortedConnections.filter { $0.protocol == "https" }
        guard !sorted.isEmpty else { return (nil, false) }

        let result: PlexServerConnection? = await withTaskGroup(of: (Int, Bool).self) { group in
            for (i, connection) in sorted.enumerated() {
                group.addTask {
                    let ok = await self.testConnection(uri: connection.uri, token: server.accessToken)
                    return (i, ok)
                }
            }

            var bestIndex: Int?
            for await (index, succeeded) in group {
                guard succeeded else { continue }
                if let current = bestIndex {
                    bestIndex = min(current, index)
                } else {
                    bestIndex = index
                }
                // If the highest-priority connection succeeded, no need to wait for others
                if bestIndex == 0 { break }
            }

            guard let idx = bestIndex else { return nil }
            return sorted[idx]
        }

        guard let connection = result else { return (nil, false) }
        let isHTTP = connection.protocol == "http"
        if isHTTP {
            logger.warning("best available connection uses plaintext HTTP — token will be sent unencrypted (uri: \(connection.uri, privacy: .private))")
        }
        return (connection, isHTTP)
    }

    // MARK: - Server Connection

    /// Connect to a Plex server and verify connectivity.
    public func connect(serverURL: URL, token: String) async throws {
        _state.withLock {
            $0.serverURL = serverURL
            $0.token = token
        }

        // Verify by fetching server identity
        let url = serverURL.appendingPathComponent("identity")
        var request = URLRequest(url: url)
        applyStandardHeaders(to: &request, token: token)

        let (_, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw PlexClientError.connectionFailed
        }
    }

    // MARK: - Library

    /// Fetch library sections from the connected server.
    private func fetchLibraries() async throws -> [LibrarySection] {
        let data = try await get(path: "library/sections")
        let container = try JSONDecoder().decode(LibrarySectionsResponse.self, from: data)
        return container.mediaContainer.directory ?? []
    }

    /// Find all music-type library sections (type == "artist").
    public func findMusicLibraries() async throws -> [LibrarySection] {
        let libraries = try await fetchLibraries()
        let music = libraries.filter { $0.type == "artist" }
        guard !music.isEmpty else { throw PlexClientError.noMusicLibrary }
        return music
    }

    // MARK: - Paginated Fetching (for sync)

    /// Fetch all items of a given type from a library section, paginated.
    /// type: 8=artist, 9=album, 10=track
    public func fetchAllItems(libraryKey: String, type: Int, pageSize: Int = 200) async throws -> [MediaItem] {
        var allItems: [MediaItem] = []
        var offset = 0
        let maxPages = 5000 // safety limit: 5000 × 200 = 1M items

        for _ in 0..<maxPages {
            let queryItems = [
                URLQueryItem(name: "type", value: "\(type)"),
                URLQueryItem(name: "X-Plex-Container-Start", value: "\(offset)"),
                URLQueryItem(name: "X-Plex-Container-Size", value: "\(pageSize)"),
            ]
            let data = try await get(path: "library/sections/\(libraryKey)/all", queryItems: queryItems)
            let container = try JSONDecoder().decode(MediaContainerResponse.self, from: data)
            let items = container.mediaContainer.metadata ?? []

            if items.isEmpty { break }
            allItems.append(contentsOf: items)

            if items.count < pageSize { break }
            offset += pageSize
        }
        return allItems
    }

    /// Fetch full metadata for a single item (album, artist, track).
    /// This returns ALL genres, unlike list endpoints which return only 1.
    public func fetchItemMetadata(ratingKey: String) async throws -> MediaItem {
        let data = try await get(path: "library/metadata/\(ratingKey)")
        let container = try JSONDecoder().decode(MediaContainerResponse.self, from: data)
        guard let item = container.mediaContainer.metadata?.first else {
            throw PlexClientError.invalidResponse
        }
        return item
    }

    // MARK: - Lyrics

    /// Find a lyrics stream (streamType 4) from full track metadata.
    public func fetchLyricsStream(ratingKey: String) async throws -> StreamInfo? {
        let item = try await fetchItemMetadata(ratingKey: ratingKey)
        return item.media?.first?.parts?.first?.streams?.first(where: { $0.streamType == 4 })
    }

    /// Download raw lyrics data from a Plex stream path.
    /// Uses `Accept: */*` instead of `application/json` since lyrics may be raw LRC/TXT.
    public func downloadLyricsData(path: String) async throws -> Data {
        let (serverURL, token) = _state.withLock { ($0.serverURL, $0.token) }
        guard let serverURL, let token else { throw PlexClientError.notConnected }
        var components = URLComponents(url: serverURL.appendingPathComponent(path), resolvingAgainstBaseURL: false)!
        components.queryItems = [URLQueryItem(name: "X-Plex-Token", value: token)]
        var request = URLRequest(url: components.url!)
        applyStandardHeaders(to: &request)
        request.setValue("*/*", forHTTPHeaderField: "Accept") // lyrics may be raw LRC/TXT
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode) else {
            throw PlexClientError.httpError((response as? HTTPURLResponse)?.statusCode ?? 0)
        }
        return data
    }

    // MARK: - Audio Levels (Waveform)

    /// Fetch loudness level samples for an audio stream (for waveform visualization).
    /// Requires the Plex server to have "Analyze audio tracks for loudness" enabled.
    public func fetchLevels(streamID: Int, subsample: Int = 600) async throws -> [Float] {
        let data = try await get(
            path: "library/streams/\(streamID)/levels",
            queryItems: [URLQueryItem(name: "subsample", value: "\(subsample)")]
        )
        let response = try JSONDecoder().decode(LevelsResponse.self, from: data)
        return response.mediaContainer.level?.map(\.v) ?? []
    }

    // MARK: - Timeline Reporting

    /// Report playback timeline to the server so it appears on the Plex dashboard.
    /// Must be called on state changes (play/pause/stop) and periodically (~10s) during playback.
    public func reportTimeline(
        ratingKey: String,
        state: String,
        timeMs: Int,
        durationMs: Int,
        sessionIdentifier: String
    ) async {
        let (serverURL, token) = _state.withLock { ($0.serverURL, $0.token) }
        guard let serverURL, let token else { return }

        let queryItems = [
            URLQueryItem(name: "ratingKey", value: ratingKey),
            URLQueryItem(name: "key", value: "/library/metadata/\(ratingKey)"),
            URLQueryItem(name: "state", value: state),
            URLQueryItem(name: "time", value: "\(timeMs)"),
            URLQueryItem(name: "duration", value: "\(durationMs)"),
            URLQueryItem(name: "identifier", value: "com.plexapp.plugins.library"),
            URLQueryItem(name: "X-Plex-Token", value: token),
        ]

        var components = URLComponents(
            url: serverURL.appendingPathComponent("/:/timeline"),
            resolvingAgainstBaseURL: false
        )!
        components.queryItems = queryItems

        var request = URLRequest(url: components.url!)
        applyStandardHeaders(to: &request)
        request.setValue(sessionIdentifier, forHTTPHeaderField: "X-Plex-Session-Identifier")

        _ = try? await URLSession.shared.data(for: request)
    }

    /// Mark an item as played on the server.
    public func scrobble(ratingKey: String) async {
        let (serverURL, token) = _state.withLock { ($0.serverURL, $0.token) }
        guard let serverURL, let token else { return }

        var components = URLComponents(
            url: serverURL.appendingPathComponent("/:/scrobble"),
            resolvingAgainstBaseURL: false
        )!
        components.queryItems = [
            URLQueryItem(name: "key", value: "/library/metadata/\(ratingKey)"),
            URLQueryItem(name: "identifier", value: "com.plexapp.plugins.library"),
            URLQueryItem(name: "X-Plex-Token", value: token),
        ]

        var request = URLRequest(url: components.url!)
        applyStandardHeaders(to: &request)

        _ = try? await URLSession.shared.data(for: request)
    }

    // MARK: - Rating

    /// Set the user rating on an item (album or track).
    /// Use rating=10 for favourite, rating=0 to unfavourite.
    public func rateItem(ratingKey: String, rating: Double) async throws {
        try await put(path: ":/rate", queryItems: [
            URLQueryItem(name: "key", value: ratingKey),
            URLQueryItem(name: "identifier", value: "com.plexapp.plugins.library"),
            URLQueryItem(name: "rating", value: "\(rating)"),
        ])
    }

    // MARK: - HTTP

    /// Apply standard Plex headers (Accept: JSON, Client-Identifier, Product, Platform, Device).
    private func applyStandardHeaders(to request: inout URLRequest, token: String? = nil) {
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue(clientIdentifier, forHTTPHeaderField: "X-Plex-Client-Identifier")
        request.setValue("ramus", forHTTPHeaderField: "X-Plex-Product")
        request.setValue("macOS", forHTTPHeaderField: "X-Plex-Platform")
        request.setValue("Mac", forHTTPHeaderField: "X-Plex-Device")
        if let token { request.setValue(token, forHTTPHeaderField: "X-Plex-Token") }
    }

    private func withRetry<T>(_ work: () async throws -> T) async throws -> T {
        do {
            return try await work()
        } catch {
            if isConnectionError(error), let reconnect = onRequestFailed, await reconnect() {
                return try await work()
            }
            throw error
        }
    }

    private func put(path: String, queryItems: [URLQueryItem] = []) async throws {
        try await withRetry {
            let (serverURL, token) = self._state.withLock { ($0.serverURL, $0.token) }
            guard let serverURL, let token else { throw PlexClientError.notConnected }

            var components = URLComponents(url: serverURL.appendingPathComponent(path), resolvingAgainstBaseURL: false)!
            var allItems = queryItems
            allItems.append(URLQueryItem(name: "X-Plex-Token", value: token))
            components.queryItems = allItems

            var request = URLRequest(url: components.url!)
            request.httpMethod = "PUT"
            self.applyStandardHeaders(to: &request)

            let (_, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse else {
                throw PlexClientError.invalidResponse
            }
            if http.statusCode == 401 { throw PlexClientError.unauthorized }
            guard (200...299).contains(http.statusCode) else {
                throw PlexClientError.httpError(http.statusCode)
            }
        }
    }

    /// Maximum response size (50 MB) — rejects absurdly large payloads before JSON parsing.
    private static let maxResponseBytes = 50 * 1024 * 1024

    private func get(path: String, queryItems: [URLQueryItem] = []) async throws -> Data {
        try await withRetry {
            let (serverURL, token) = self._state.withLock { ($0.serverURL, $0.token) }
            guard let serverURL, let token else { throw PlexClientError.notConnected }

            var components = URLComponents(url: serverURL.appendingPathComponent(path), resolvingAgainstBaseURL: false)!
            var allItems = queryItems
            allItems.append(URLQueryItem(name: "X-Plex-Token", value: token))
            components.queryItems = allItems

            var request = URLRequest(url: components.url!)
            self.applyStandardHeaders(to: &request)

            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse else {
                throw PlexClientError.invalidResponse
            }
            if http.statusCode == 401 { throw PlexClientError.unauthorized }
            guard (200...299).contains(http.statusCode) else {
                throw PlexClientError.httpError(http.statusCode)
            }
            guard data.count <= Self.maxResponseBytes else {
                logger.error("response too large (\(data.count, privacy: .public) bytes) for \(path, privacy: .private)")
                throw PlexClientError.invalidResponse
            }
            return data
        }
    }

    private func isConnectionError(_ error: Error) -> Bool {
        if let urlError = error as? URLError {
            return [.notConnectedToInternet, .networkConnectionLost, .timedOut,
                    .cannotFindHost, .cannotConnectToHost, .dnsLookupFailed,
                    .secureConnectionFailed].contains(urlError.code)
        }
        return false
    }
}

// MARK: - Errors

public enum PlexClientError: Error, Sendable {
    case notConnected
    case connectionFailed
    case noMusicLibrary
    case invalidResponse
    case unauthorized
    case httpError(Int)
    case noSecureConnection
}

// MARK: - Response Models

public struct LibrarySection: Codable, Hashable, Sendable {
    public let key: String
    public let title: String
    public let type: String
}

struct LibrarySectionsResponse: Codable {
    let mediaContainer: LibrarySectionsContainer

    enum CodingKeys: String, CodingKey {
        case mediaContainer = "MediaContainer"
    }
}

struct LibrarySectionsContainer: Codable {
    let directory: [LibrarySection]?

    enum CodingKeys: String, CodingKey {
        case directory = "Directory"
    }
}

public struct MediaContainerResponse: Codable, Sendable {
    public let mediaContainer: MediaContainer

    enum CodingKeys: String, CodingKey {
        case mediaContainer = "MediaContainer"
    }
}

public struct MediaContainer: Codable, Sendable {
    public let metadata: [MediaItem]?

    enum CodingKeys: String, CodingKey {
        case metadata = "Metadata"
    }
}

public struct MediaItem: Codable, Sendable {
    public let ratingKey: String
    public let title: String
    public let titleSort: String?
    public let originalTitle: String?
    public let summary: String?
    public let parentTitle: String?
    public let grandparentTitle: String?
    public let parentRatingKey: String?
    public let grandparentRatingKey: String?
    public let index: Int?
    public let parentIndex: Int?
    public let year: Int?
    public let duration: Int?
    public let updatedAt: Int?
    public let addedAt: Int?
    public let lastViewedAt: Int?
    public let thumb: String?
    public let parentThumb: String?
    public let grandparentThumb: String?
    public let art: String?
    public let userRating: Double?
    public let studio: String?
    public let media: [MediaInfo]?
    public let genre: [PlexTag]?
    public let ultraBlurColors: UltraBlurColors?

    enum CodingKeys: String, CodingKey {
        case ratingKey, title, titleSort, originalTitle, summary
        case parentTitle, grandparentTitle
        case parentRatingKey, grandparentRatingKey
        case index, parentIndex, year, duration, updatedAt, addedAt, lastViewedAt
        case thumb, parentThumb, grandparentThumb, art
        case userRating, studio
        case media = "Media"
        case genre = "Genre"
        case ultraBlurColors = "UltraBlurColors"
    }
}

public struct PlexTag: Codable, Sendable {
    public let tag: String
}

public struct MediaInfo: Codable, Sendable {
    public let audioCodec: String?
    public let bitrate: Int?
    public let parts: [PartInfo]?

    enum CodingKeys: String, CodingKey {
        case audioCodec, bitrate
        case parts = "Part"
    }
}

public struct PartInfo: Codable, Sendable {
    public let key: String?
    public let streams: [StreamInfo]?

    enum CodingKeys: String, CodingKey {
        case key
        case streams = "Stream"
    }
}

public struct StreamInfo: Codable, Sendable {
    public let id: Int?
    public let streamType: Int?
    public let codec: String?
    public let bitrate: Int?
    // Lyrics stream fields (streamType == 4)
    public let key: String?
    public let format: String?
    public let timed: Bool?
    public let provider: String?

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(Int.self, forKey: .id)
        streamType = try c.decodeIfPresent(Int.self, forKey: .streamType)
        codec = try c.decodeIfPresent(String.self, forKey: .codec)
        bitrate = try c.decodeIfPresent(Int.self, forKey: .bitrate)
        key = try c.decodeIfPresent(String.self, forKey: .key)
        format = try c.decodeIfPresent(String.self, forKey: .format)
        provider = try c.decodeIfPresent(String.self, forKey: .provider)
        // Plex returns timed as bool for lyrics streams, but could be int elsewhere
        if let b = try? c.decodeIfPresent(Bool.self, forKey: .timed) {
            timed = b
        } else if let i = try? c.decodeIfPresent(Int.self, forKey: .timed) {
            timed = i != 0
        } else {
            timed = nil
        }
    }

    private enum CodingKeys: String, CodingKey {
        case id, streamType, codec, bitrate, key, format, timed, provider
    }
}

// MARK: - Levels Response (Waveform)

struct LevelsResponse: Codable, Sendable {
    let mediaContainer: LevelsContainer

    enum CodingKeys: String, CodingKey {
        case mediaContainer = "MediaContainer"
    }
}

struct LevelsContainer: Codable, Sendable {
    let level: [LevelSample]?

    enum CodingKeys: String, CodingKey {
        case level = "Level"
    }
}

struct LevelSample: Codable, Sendable {
    let v: Float
}

// MARK: - Plex.tv Resources API Response

struct PlexResourceResponse: Codable {
    let name: String
    let provides: String
    let clientIdentifier: String
    let accessToken: String?
    let owned: Bool?
    let connections: [PlexResourceConnectionResponse]?
}

struct PlexResourceConnectionResponse: Codable {
    let uri: String
    let local: Bool?
    let relay: Bool?
    let address: String?
    let port: Int?
    let `protocol`: String?
}
