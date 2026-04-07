import Foundation

/// Plex media identifier (ratingKey string).
public typealias PlexID = String

/// Duration in seconds.
public typealias Duration = TimeInterval

// MARK: - Range Operators (used by Search + Cache)

public enum RangeOp: Sendable, Hashable {
    case equal
    case greaterThan
    case lessThan
    case greaterOrEqual
    case lessOrEqual

    /// SQL comparison operator literal. Closed set — no injection risk.
    public var sqlLiteral: String {
        switch self {
        case .equal: "="
        case .greaterThan: ">"
        case .lessThan: "<"
        case .greaterOrEqual: ">="
        case .lessOrEqual: "<="
        }
    }
}

public enum RangeField: Sendable, Hashable {
    case year
    case rating
}

// MARK: - UltraBlur Colors

/// Four hex-colour strings for UltraBlurBackground corners.
/// Pure data — no UI or API dependency.
public struct UltraBlurColors: Codable, Sendable, Hashable {
    public let topLeft: String
    public let topRight: String
    public let bottomRight: String
    public let bottomLeft: String

    public init(topLeft: String, topRight: String, bottomRight: String, bottomLeft: String) {
        self.topLeft = topLeft
        self.topRight = topRight
        self.bottomRight = bottomRight
        self.bottomLeft = bottomLeft
    }
}

// MARK: - Album

public struct Album: Codable, Hashable, Identifiable, Sendable {
    public var id: PlexID { ratingKey }
    public let ratingKey: PlexID
    public let title: String
    public let artistName: String
    public let year: Int?
    public let thumb: String?
    public let genres: [String]
    public let isFavourite: Bool
    public let studio: String?
    public let addedAt: Int?
    public let lastViewedAt: Int?

    public init(
        ratingKey: PlexID,
        title: String,
        artistName: String,
        year: Int? = nil,
        thumb: String? = nil,
        genres: [String] = [],
        isFavourite: Bool = false,
        studio: String? = nil,
        addedAt: Int? = nil,
        lastViewedAt: Int? = nil
    ) {
        self.ratingKey = ratingKey
        self.title = title
        self.artistName = artistName
        self.year = year
        self.thumb = thumb
        self.genres = genres
        self.isFavourite = isFavourite
        self.studio = studio
        self.addedAt = addedAt
        self.lastViewedAt = lastViewedAt
    }
}

// MARK: - Track

public struct Track: Codable, Hashable, Identifiable, Sendable {
    public var id: PlexID { ratingKey }
    public let ratingKey: PlexID
    public let title: String
    public let artistName: String
    public let trackArtist: String?
    public let albumTitle: String
    public let albumKey: PlexID?
    public let index: Int?
    public let duration: Duration
    public let codec: String?
    public let partKey: String?
    public let thumb: String?
    public let isFavourite: Bool
    public let bitrate: Int?
    public let discNumber: Int?

    /// The artist to display for this track: track-level override if present, otherwise album artist.
    public var displayArtist: String {
        guard let ta = trackArtist, !ta.isEmpty else { return artistName }
        return ta
    }

    /// True when this track has a different artist from the album artist.
    public var hasTrackArtist: Bool {
        guard let ta = trackArtist, !ta.isEmpty else { return false }
        return ta.lowercased() != artistName.lowercased()
    }

    /// Audio format display: "FLAC" for lossless, "MP3 320 kbps" for lossy.
    public var formatDescription: String? {
        guard let codec else { return nil }
        let lossless: Set<String> = ["flac", "alac", "wav", "aiff", "pcm"]
        if lossless.contains(codec.lowercased()) {
            return codec.uppercased()
        }
        if let bitrate {
            return "\(codec.uppercased()) \(bitrate) kbps"
        }
        return codec.uppercased()
    }

    public init(
        ratingKey: PlexID,
        title: String,
        artistName: String,
        trackArtist: String? = nil,
        albumTitle: String,
        albumKey: PlexID? = nil,
        index: Int? = nil,
        duration: Duration = 0,
        codec: String? = nil,
        partKey: String? = nil,
        thumb: String? = nil,
        isFavourite: Bool = false,
        bitrate: Int? = nil,
        discNumber: Int? = nil
    ) {
        self.ratingKey = ratingKey
        self.title = title
        self.artistName = artistName
        self.trackArtist = trackArtist
        self.albumTitle = albumTitle
        self.albumKey = albumKey
        self.index = index
        self.duration = duration
        self.codec = codec
        self.partKey = partKey
        self.thumb = thumb
        self.isFavourite = isFavourite
        self.bitrate = bitrate
        self.discNumber = discNumber
    }
}

// MARK: - Server Discovery

public struct PlexServerConnection: Codable, Hashable, Sendable {
    public let uri: String
    public let local: Bool
    public let relay: Bool
    public let `protocol`: String

    /// Priority: lower = preferred. HTTPS connections rank above HTTP within each tier.
    /// 0=local HTTPS, 1=remote HTTPS, 2=relay HTTPS, 3=local HTTP, 4=remote HTTP, 5=relay HTTP
    public var priority: Int {
        let https = `protocol` == "https"
        if local { return https ? 0 : 3 }
        if !relay { return https ? 1 : 4 }
        return https ? 2 : 5
    }

    public init(uri: String, local: Bool, relay: Bool, protocol: String) {
        self.uri = uri
        self.local = local
        self.relay = relay
        self.protocol = `protocol`
    }
}

public struct PlexServer: Codable, Hashable, Identifiable, Sendable {
    public var id: String { machineIdentifier }
    public let machineIdentifier: String
    public let name: String
    public let accessToken: String
    public let owned: Bool
    public let connections: [PlexServerConnection]

    public var sortedConnections: [PlexServerConnection] {
        connections.sorted { $0.priority < $1.priority }
    }

    public init(machineIdentifier: String, name: String, accessToken: String, owned: Bool, connections: [PlexServerConnection]) {
        self.machineIdentifier = machineIdentifier
        self.name = name
        self.accessToken = accessToken
        self.owned = owned
        self.connections = connections
    }
}

public struct ServerConfig: Codable, Sendable {
    public var machineIdentifier: String
    public var name: String
    public var accessToken: String
    public var selectedLibraryKey: String?

    private enum CodingKeys: String, CodingKey {
        case machineIdentifier, name, accessToken, selectedLibraryKey
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(machineIdentifier, forKey: .machineIdentifier)
        try container.encode(name, forKey: .name)
        // accessToken excluded from encoding — stored in encrypted token file, not serialized
        try container.encodeIfPresent(selectedLibraryKey, forKey: .selectedLibraryKey)
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        machineIdentifier = try container.decode(String.self, forKey: .machineIdentifier)
        name = try container.decode(String.self, forKey: .name)
        // Decode if present for backward compat (migration data); defaults to empty
        accessToken = (try? container.decode(String.self, forKey: .accessToken)) ?? ""
        selectedLibraryKey = try container.decodeIfPresent(String.self, forKey: .selectedLibraryKey)
    }

    public init(machineIdentifier: String, name: String, accessToken: String, selectedLibraryKey: String? = nil) {
        self.machineIdentifier = machineIdentifier
        self.name = name
        self.accessToken = accessToken
        self.selectedLibraryKey = selectedLibraryKey
    }
}

// MARK: - PlayerState

public enum PlaybackStatus: Equatable, Sendable {
    case playing
    case paused
    case stopped
}

public struct PlayerState: Sendable {
    public var status: PlaybackStatus
    public var currentTrack: Track?
    public var queue: [Track]
    public var queueIndex: Int

    public init(
        status: PlaybackStatus = .stopped,
        currentTrack: Track? = nil,
        queue: [Track] = [],
        queueIndex: Int = 0
    ) {
        self.status = status
        self.currentTrack = currentTrack
        self.queue = queue
        self.queueIndex = queueIndex
    }
}
