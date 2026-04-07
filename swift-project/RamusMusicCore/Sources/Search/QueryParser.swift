import Foundation
import Models

// MARK: - Types

public enum SearchFilter: Sendable, Hashable {
    case freeText(String)        // plain text → search albums (artist + album title)
    case genre(String)           // /metal → albums in genre
    case artist(String)          // @radiohead → albums by artist
    case albumTitle(String)      // %ok computer → albums by title
    case trackSearch(String)     // !paranoid → tracks by title
    case range(RangeField, RangeOp, Double)  // year:>2000, rating:>=8
    case favourites                          // fav: → filter to favourited albums
}

public struct ParsedQuery: Sendable, Hashable {
    public let filters: [SearchFilter]
    public var isEmpty: Bool { filters.isEmpty }

    public var freeText: String? {
        filters.compactMap {
            if case .freeText(let t) = $0 { return t }
            return nil
        }.first
    }

    public var genreFilters: [String] {
        filters.compactMap {
            if case .genre(let g) = $0 { return g }
            return nil
        }
    }

    public var artistFilters: [String] {
        filters.compactMap {
            if case .artist(let a) = $0 { return a }
            return nil
        }
    }

    public var albumTitleFilters: [String] {
        filters.compactMap {
            if case .albumTitle(let a) = $0 { return a }
            return nil
        }
    }

    public var trackSearches: [String] {
        filters.compactMap {
            if case .trackSearch(let t) = $0 { return t }
            return nil
        }
    }

    public var rangeFilters: [(RangeField, RangeOp, Double)] {
        filters.compactMap {
            if case .range(let f, let op, let v) = $0 { return (f, op, v) }
            return nil
        }
    }

    /// Whether this query explicitly requests track results (! operator).
    public var hasTrackSearch: Bool { !trackSearches.isEmpty }

    /// Whether this query includes a favourites filter.
    public var hasFavouritesFilter: Bool {
        filters.contains { if case .favourites = $0 { return true }; return false }
    }

    /// Whether this query contains only a single free-text filter with no operators.
    public var isFreeTextOnly: Bool {
        filters.count == 1 && freeText != nil
    }
}

// MARK: - Parser

public enum QueryParser {

    /// Parse a raw query string into structured filters.
    /// Each operator consumes all text until an explicit `AND` delimiter.
    /// e.g. `@blue öyster cult AND $>1970` → artist("blue öyster cult") + range(year, >, 1970)
    /// Without `AND`, the entire input belongs to the first operator:
    /// e.g. `/rock year:2000` → genre("rock year:2000")
    public static func parse(_ input: String) -> ParsedQuery {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return ParsedQuery(filters: []) }

        // Split on " AND " (case-sensitive, uppercase to avoid collision with band names)
        let segments = trimmed.components(separatedBy: " AND ")

        var filters: [SearchFilter] = []

        for segment in segments {
            let s = segment.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !s.isEmpty else { continue }

            if let filter = parseSegment(s) {
                filters.append(filter)
            }
        }

        return ParsedQuery(filters: filters)
    }

    /// Strip FTS5 metacharacters from a string for safe use in MATCH queries.
    /// Drops `"`, `*`, `(`, `)`, `:`, `^`, `{`, `}` (FTS5 operators and grouping).
    /// Replaces `-` with space (FTS5 NOT operator; also aligns with unicode61 tokenizer
    /// which treats hyphens as word separators).
    public static func escapeFTS5(_ text: String) -> String {
        var result = ""
        for char in text {
            switch char {
            case "\"", "*", "(", ")", ":", "^", "{", "}":
                continue
            case "-":
                result.append(" ")
            default:
                result.append(char)
            }
        }
        return result
    }

    // MARK: - Private

    /// Parse a single segment (text between AND delimiters) into a filter.
    private static func parseSegment(_ segment: String) -> SearchFilter? {
        if segment.hasPrefix("/") {
            let value = String(segment.dropFirst()).trimmingCharacters(in: .whitespaces)
            return value.isEmpty ? nil : .genre(value)
        } else if segment.hasPrefix("@") {
            let value = String(segment.dropFirst()).trimmingCharacters(in: .whitespaces)
            return value.isEmpty ? nil : .artist(value)
        } else if segment.hasPrefix("!") {
            let value = String(segment.dropFirst()).trimmingCharacters(in: .whitespaces)
            return value.isEmpty ? nil : .trackSearch(value)
        } else if segment.hasPrefix("%") {
            let value = String(segment.dropFirst()).trimmingCharacters(in: .whitespaces)
            return value.isEmpty ? nil : .albumTitle(value)
        } else if segment.lowercased().hasPrefix("fav:") || segment.lowercased().hasPrefix("favourites:") {
            return .favourites
        } else if segment.hasPrefix("$") {
            let remainder = String(segment.dropFirst())
            return parseRange(.year, remainder) ?? .freeText(segment)
        } else if segment.lowercased().hasPrefix("year:") {
            let remainder = String(segment.dropFirst(5))
            return parseRange(.year, remainder) ?? .freeText(segment)
        } else if segment.lowercased().hasPrefix("rating:") {
            let remainder = String(segment.dropFirst(7))
            return parseRange(.rating, remainder) ?? .freeText(segment)
        } else {
            return .freeText(segment)
        }
    }

    private static func parseRange(_ field: RangeField, _ value: String) -> SearchFilter? {
        let op: RangeOp
        let numStr: String

        if value.hasPrefix(">=") {
            op = .greaterOrEqual
            numStr = String(value.dropFirst(2))
        } else if value.hasPrefix("<=") {
            op = .lessOrEqual
            numStr = String(value.dropFirst(2))
        } else if value.hasPrefix(">") {
            op = .greaterThan
            numStr = String(value.dropFirst(1))
        } else if value.hasPrefix("<") {
            op = .lessThan
            numStr = String(value.dropFirst(1))
        } else {
            op = .equal
            numStr = value
        }

        guard let number = Double(numStr) else { return nil }
        return .range(field, op, number)
    }
}
