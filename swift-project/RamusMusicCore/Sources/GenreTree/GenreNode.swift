import Foundation

/// A node in the genre hierarchy tree.
/// `children` is nil for leaf nodes (required for SwiftUI List/DisclosureGroup).
/// `id` is path-based (e.g. "rock/funk") to handle genres appearing in multiple subtrees.
public struct GenreNode: Identifiable, Hashable, Sendable {
    public let id: String
    public let name: String
    public let shortSummary: String?
    public var children: [GenreNode]?
    public var albumCount: Int
    public var deduplicatedTotalCount: Int

    public init(name: String, parentPath: String = "", shortSummary: String? = nil, children: [GenreNode]? = nil, albumCount: Int = 0, deduplicatedTotalCount: Int = 0) {
        self.id = parentPath.isEmpty ? name.lowercased() : "\(parentPath)/\(name.lowercased())"
        self.name = name
        self.shortSummary = shortSummary
        self.children = children
        self.albumCount = albumCount
        self.deduplicatedTotalCount = deduplicatedTotalCount
    }

    /// All genre names in this subtree (self + all descendants), flattened.
    public var allDescendantNames: [String] {
        var result = [name]
        if let children {
            for child in children {
                result.append(contentsOf: child.allDescendantNames)
            }
        }
        return result
    }

    /// Collect all genre names in this subtree directly into a Set, avoiding intermediate arrays.
    public func collectDescendantNames(into set: inout Set<String>) {
        set.insert(name)
        if let children {
            for child in children {
                child.collectDescendantNames(into: &set)
            }
        }
    }
}
