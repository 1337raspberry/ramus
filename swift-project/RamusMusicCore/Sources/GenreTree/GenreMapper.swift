import Foundation
import os
@preconcurrency import Fuse

/// Maps flat Plex genre strings to a genre hierarchy.
/// Loads a genre JSON file from a bundle, builds a lookup table, and provides
/// fuzzy matching via Fuse for slight naming differences.
public final class GenreMapper: Sendable {

    /// The full hierarchy as loaded from JSON.
    public let rootNodes: [GenreNode]

    /// Case-insensitive lookup: lowercased genre name → GenreNode
    private let exactLookup: [String: GenreNode]

    /// All genre names for fuzzy search.
    private let allNames: [String]

    /// Fuse instance for fuzzy matching.
    private let fuse: Fuse

    /// Cache for matchGenre results — avoids repeated Fuse searches for the same input.
    /// Protected by OSAllocatedUnfairLock because SyncEngine + SearchEngine call
    /// matchGenre concurrently from different isolation contexts.
    private let cacheLock = OSAllocatedUnfairLock(initialState: MatchCacheState())

    private struct MatchCacheState: Sendable {
        var matches: [String: GenreNode] = [:]
        var misses: Set<String> = []
    }

    // MARK: - Init

    /// Load from a genre hierarchy JSON file URL.
    public init(jsonURL: URL) throws {
        let data = try Data(contentsOf: jsonURL)
        let raw = try JSONDecoder().decode(GenreFile.self, from: data)
        let nodes = raw.genres.map { Self.convertRawNode($0, parentPath: "") }
        self.rootNodes = nodes

        var lookup: [String: GenreNode] = [:]
        Self.buildLookup(nodes: nodes, into: &lookup)
        self.exactLookup = lookup
        self.allNames = Array(lookup.keys).map { lookup[$0]!.name }
        self.fuse = Fuse(threshold: 0.4)
    }

    /// Load from a bundle (looks for a named JSON resource).
    public convenience init(bundle: Bundle = .main, resourceName: String = "open") throws {
        guard let url = bundle.url(forResource: resourceName, withExtension: "json") else {
            throw GenreMapperError.resourceNotFound(resourceName)
        }
        try self.init(jsonURL: url)
    }

    // MARK: - Matching

    /// Match a Plex genre string to the genre hierarchy.
    /// Tries exact (case-insensitive) first, then fuzzy.
    /// Results are cached so each unique genre string only fuzzy-searches once.
    public func matchGenre(_ plexGenre: String) -> GenreNode? {
        let key = plexGenre.lowercased()

        // Check caches first (lock protects concurrent reads/writes)
        let (cachedMatch, isKnown) = cacheLock.withLock { state -> (GenreNode?, Bool) in
            if let match = state.matches[key] { return (match, true) }
            if state.misses.contains(key) { return (nil, true) }
            return (nil, false)
        }
        if isKnown { return cachedMatch }

        // Exact match
        if let node = exactLookup[key] {
            cacheLock.withLock { $0.matches[key] = node }
            return node
        }

        // Fuzzy fallback via Fuse (expensive — runs outside lock)
        guard let pattern = fuse.createPattern(from: plexGenre) else {
            cacheLock.withLock { _ = $0.misses.insert(key) }
            return nil
        }

        var bestScore = Double.infinity
        var bestName: String?

        for name in allNames {
            if let result = fuse.search(pattern, in: name) {
                if result.score < bestScore {
                    bestScore = result.score
                    bestName = name
                }
            }
        }

        if let bestName, let node = exactLookup[bestName.lowercased()] {
            cacheLock.withLock { $0.matches[key] = node }
            return node
        }

        cacheLock.withLock { _ = $0.misses.insert(key) }
        return nil
    }

    /// Build a display tree from album sets, pruning empty branches and computing
    /// deduplicated subtree counts via set unions.
    /// `genreAlbumSets` maps genre name → set of album IDs.
    public func buildDisplayTree(genreAlbumSets: [String: Set<Int64>]) -> [GenreNode] {
        let lowered = Dictionary(uniqueKeysWithValues: genreAlbumSets.map { ($0.key.lowercased(), $0.value) })
        var matchedNames = Set<String>()
        var pruned = rootNodes.compactMap { pruneNode($0, albumSets: lowered, parentPath: "", matchedNames: &matchedNames) }
        // Post-order traversal to compute deduplicated counts
        for i in pruned.indices {
            _ = computeDeduplicatedCounts(&pruned[i], albumSets: lowered)
        }

        // Collect unmatched genres into an "Other" node
        let unmatched = lowered.filter { !matchedNames.contains($0.key) }
        if !unmatched.isEmpty {
            let otherChildren = unmatched.keys.sorted().map { key in
                let original = genreAlbumSets.first { $0.key.lowercased() == key }
                let name = original?.key ?? key
                let count = original?.value.count ?? 0
                return GenreNode(name: name, parentPath: "other", albumCount: count, deduplicatedTotalCount: count)
            }
            let otherUnion = unmatched.values.reduce(into: Set<Int64>()) { $0.formUnion($1) }
            let other = GenreNode(
                name: "Other", children: otherChildren,
                albumCount: 0, deduplicatedTotalCount: otherUnion.count
            )
            pruned.append(other)
        }

        return pruned
    }

    // MARK: - Private: JSON parsing

    private struct GenreFile: Decodable {
        let genres: [GenreNodeRaw]
    }

    private struct GenreNodeRaw: Decodable {
        let name: String
        let shortSummary: String?
        let children: [GenreNodeRaw]?

        enum CodingKeys: String, CodingKey {
            case name
            case shortSummary = "short_summary"
            case children
        }
    }

    private static func convertRawNode(_ raw: GenreNodeRaw, parentPath: String) -> GenreNode {
        let displayName = titleCase(raw.name)
        let nodePath = parentPath.isEmpty ? raw.name.lowercased() : "\(parentPath)/\(raw.name.lowercased())"
        let children = raw.children?.isEmpty == false
            ? raw.children!.map { convertRawNode($0, parentPath: nodePath) }
            : nil
        return GenreNode(name: displayName, parentPath: parentPath, shortSummary: raw.shortSummary, children: children)
    }

    /// Title-case a genre name for display. Words that are already capitalised
    /// or contain uppercase letters (acronyms like "EBM", "R&B") are left
    /// untouched. All-lowercase words get their first letter capitalised.
    /// Handles hyphenated compounds ("lo-fi" → "Lo-Fi").
    private static func titleCase(_ input: String) -> String {
        input.split(separator: " ", omittingEmptySubsequences: false)
            .map { word in
                // Process each hyphen-separated segment independently
                word.split(separator: "-", omittingEmptySubsequences: false)
                    .map { segment in
                        let s = String(segment)
                        guard s == s.lowercased() else { return s } // has uppercase → leave it
                        return s.prefix(1).uppercased() + s.dropFirst()
                    }
                    .joined(separator: "-")
            }
            .joined(separator: " ")
    }

    // MARK: - Private: Lookup

    private static func buildLookup(nodes: [GenreNode], into lookup: inout [String: GenreNode]) {
        for node in nodes {
            lookup[node.name.lowercased()] = node
            if let children = node.children {
                buildLookup(nodes: children, into: &lookup)
            }
        }
    }

    // MARK: - Private: Pruning

    /// Recursively prune a node: keep it only if it or any descendant has albums.
    /// Collects matched genre names (lowercased) into `matchedNames` for "Other" node computation.
    private func pruneNode(_ node: GenreNode, albumSets: [String: Set<Int64>], parentPath: String, matchedNames: inout Set<String>) -> GenreNode? {
        let directCount = albumSets[node.name.lowercased()]?.count ?? 0

        let prunedChildren: [GenreNode]? = node.children.flatMap { children in
            let kept = children.compactMap { pruneNode($0, albumSets: albumSets, parentPath: node.id, matchedNames: &matchedNames) }
            return kept.isEmpty ? nil : kept
        }

        if directCount > 0 || prunedChildren != nil {
            matchedNames.insert(node.name.lowercased())
            return GenreNode(
                name: node.name, parentPath: parentPath,
                shortSummary: node.shortSummary,
                children: prunedChildren, albumCount: directCount
            )
        }

        return nil
    }

    // MARK: - Private: Deduplicated Counts

    /// Post-order traversal: union album ID sets from all descendants, store count.
    /// Returns the unioned set for the parent to use.
    @discardableResult
    private func computeDeduplicatedCounts(_ node: inout GenreNode, albumSets: [String: Set<Int64>]) -> Set<Int64> {
        var unionSet = albumSets[node.name.lowercased()] ?? []

        if node.children != nil {
            for i in node.children!.indices {
                let childSet = computeDeduplicatedCounts(&node.children![i], albumSets: albumSets)
                unionSet.formUnion(childSet)
            }
        }

        node.deduplicatedTotalCount = unionSet.count
        return unionSet
    }
}

public enum GenreMapperError: Error, LocalizedError {
    case resourceNotFound(String)

    public var errorDescription: String? {
        switch self {
        case .resourceNotFound(let name): "\(name).json not found in bundle"
        }
    }
}
