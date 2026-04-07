import Foundation
import Models
import PlexAPI
import os.log

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "LyricsProvider")

/// A single line of lyrics, optionally with a timestamp for synced scrolling.
public struct LyricLine: Sendable, Identifiable {
    public let id: Int
    public let timestamp: TimeInterval?
    public let text: String
}

/// Where the lyrics came from.
public enum LyricsSource: Sendable {
    case plex
    case lrclib
}

/// The result of a lyrics fetch: lines, sync flag, and source.
public struct LyricsResult: Sendable {
    public let lines: [LyricLine]
    public let isSynced: Bool
    public let source: LyricsSource

    /// Find the index of the active line for a given playback position.
    public func activeLineIndex(at position: TimeInterval) -> Int? {
        guard isSynced else { return nil }
        var best: Int?
        for (i, line) in lines.enumerated() {
            guard let ts = line.timestamp else { continue }
            guard ts <= position else { break }
            best = i
        }
        return best
    }
}

/// Fetches lyrics from Plex local .lrc files, then falls back to LRCLIB.
public final class LyricsProvider: Sendable {

    public init() {}

    /// Fetch lyrics for a track. Tries Plex local files first, then LRCLIB.
    public func fetchLyrics(
        for track: Track,
        albumTitle: String,
        using client: PlexClient
    ) async -> LyricsResult? {
        // Try Plex local .lrc sidecar files (skip LyricFind — DRM-gated to official clients)
        if let result = await fetchFromPlex(track: track, client: client) {
            log.info("Lyrics from Plex for \(track.title, privacy: .public)")
            return result
        }
        // Fallback to LRCLIB
        if let result = await fetchFromLRCLIB(track: track, albumTitle: albumTitle) {
            log.info("Lyrics from LRCLIB for \(track.title, privacy: .public)")
            return result
        }
        log.info("No lyrics found for \(track.title, privacy: .public)")
        return nil
    }

    // MARK: - Plex (local .lrc files only)

    private func fetchFromPlex(track: Track, client: PlexClient) async -> LyricsResult? {
        do {
            guard let stream = try await client.fetchLyricsStream(ratingKey: track.ratingKey) else {
                return nil
            }
            // Skip LyricFind — only works for official Plex clients
            if stream.provider == "com.plexapp.agents.lyricfind" { return nil }

            guard let streamKey = stream.key else { return nil }
            let path = streamKey.hasPrefix("/") ? String(streamKey.dropFirst()) : streamKey
            // Validate path stays within expected Plex scope
            guard !path.contains(".."),
                  path.hasPrefix("library/") || path.hasPrefix("file/") else {
                log.warning("Rejected suspicious lyrics stream key: \(path, privacy: .public)")
                return nil
            }
            let data = try await client.downloadLyricsData(path: path)

            // Plex may return JSON wrapper or raw LRC/TXT
            if let json = try? JSONDecoder().decode(PlexLyricsResponse.self, from: data) {
                let lines = parsePlexJSON(json)
                if lines.isEmpty { return nil }
                let isSynced = lines.contains { $0.timestamp != nil }
                return LyricsResult(lines: lines, isSynced: isSynced, source: .plex)
            }

            guard let content = String(data: data, encoding: .utf8), !content.isEmpty else { return nil }
            if stream.format == "lrc" || stream.timed == true {
                let lines = parseLRC(content)
                if !lines.isEmpty { return LyricsResult(lines: lines, isSynced: true, source: .plex) }
            }
            let lines = parsePlain(content)
            return lines.isEmpty ? nil : LyricsResult(lines: lines, isSynced: false, source: .plex)
        } catch {
            return nil
        }
    }

    /// Parse the Plex JSON lyrics format (MediaContainer > Lyrics > Line > Span).
    private func parsePlexJSON(_ response: PlexLyricsResponse) -> [LyricLine] {
        guard let lyrics = response.mediaContainer.lyrics?.first,
              let jsonLines = lyrics.line else { return [] }
        var result: [LyricLine] = []
        for (index, line) in jsonLines.enumerated() {
            let text = line.text.trimmingCharacters(in: .whitespaces)
            guard !text.isEmpty else { continue }
            let timestamp: TimeInterval? = line.startOffset.map { Double($0) / 1000.0 }
            result.append(LyricLine(id: index, timestamp: timestamp, text: text))
        }
        return result
    }

    // MARK: - LRCLIB

    private func fetchFromLRCLIB(track: Track, albumTitle: String) async -> LyricsResult? {
        var components = URLComponents(string: "https://lrclib.net/api/get")!
        components.queryItems = [
            URLQueryItem(name: "track_name", value: track.title),
            URLQueryItem(name: "artist_name", value: track.displayArtist),
            URLQueryItem(name: "album_name", value: albumTitle),
            URLQueryItem(name: "duration", value: String(Int(track.duration))),
        ]
        guard let url = components.url else { return nil }

        var request = URLRequest(url: url)
        request.timeoutInterval = 10
        request.setValue("ramus v1.0.0 (https://github.com/1337raspberry/ramus)", forHTTPHeaderField: "Lrclib-Client")

        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                return nil
            }
            guard data.count <= 512_000 else { return nil } // 512 KB cap
            let decoded = try JSONDecoder().decode(LRCLIBResponse.self, from: data)
            if let synced = decoded.syncedLyrics, !synced.isEmpty {
                let lines = parseLRC(synced)
                if !lines.isEmpty { return LyricsResult(lines: lines, isSynced: true, source: .lrclib) }
            }
            if let plain = decoded.plainLyrics, !plain.isEmpty {
                let lines = parsePlain(plain)
                return lines.isEmpty ? nil : LyricsResult(lines: lines, isSynced: false, source: .lrclib)
            }
            return nil
        } catch {
            return nil
        }
    }

    // MARK: - LRC Parsing

    /// Parse LRC format: `[MM:SS.cc] text`
    private func parseLRC(_ content: String) -> [LyricLine] {
        let pattern = #"\[(\d+):(\d+(?:\.\d+)?)\]\s*(.*)"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return [] }

        var lines: [LyricLine] = []
        for (index, rawLine) in content.components(separatedBy: .newlines).enumerated() {
            let nsLine = rawLine as NSString
            let range = NSRange(location: 0, length: nsLine.length)
            guard let match = regex.firstMatch(in: rawLine, range: range) else { continue }

            let minutes = Double(nsLine.substring(with: match.range(at: 1))) ?? 0
            let seconds = Double(nsLine.substring(with: match.range(at: 2))) ?? 0
            let text = nsLine.substring(with: match.range(at: 3))
                .trimmingCharacters(in: .whitespaces)

            guard !text.isEmpty else { continue }
            lines.append(LyricLine(id: index, timestamp: minutes * 60 + seconds, text: text))
        }
        return lines
    }

    /// Parse plain text lyrics (one line per line).
    private func parsePlain(_ content: String) -> [LyricLine] {
        content.components(separatedBy: .newlines)
            .enumerated()
            .compactMap { index, line in
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                return trimmed.isEmpty ? nil : LyricLine(id: index, timestamp: nil, text: trimmed)
            }
    }
}

// MARK: - Plex Lyrics JSON Response

private struct PlexLyricsResponse: Codable {
    let mediaContainer: PlexLyricsContainer
    enum CodingKeys: String, CodingKey { case mediaContainer = "MediaContainer" }
}

private struct PlexLyricsContainer: Codable {
    let lyrics: [PlexLyricsEntry]?
    enum CodingKeys: String, CodingKey { case lyrics = "Lyrics" }
}

private struct PlexLyricsEntry: Codable {
    let line: [PlexLyricLine]?
    enum CodingKeys: String, CodingKey { case line = "Line" }
}

private struct PlexLyricLine: Codable {
    let span: [PlexLyricSpan]?
    let startOffset: Int?
    enum CodingKeys: String, CodingKey { case span = "Span"; case startOffset }

    var text: String {
        span?.compactMap(\.text).joined(separator: " ") ?? ""
    }
}

private struct PlexLyricSpan: Codable {
    let text: String?
}

// MARK: - LRCLIB Response

private struct LRCLIBResponse: Codable {
    let plainLyrics: String?
    let syncedLyrics: String?
}
