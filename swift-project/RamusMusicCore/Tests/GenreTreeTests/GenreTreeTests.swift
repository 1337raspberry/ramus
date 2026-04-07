import XCTest
@testable import GenreTree

final class GenreTreeTests: XCTestCase {

    // Helper: create a mapper from a small inline JSON
    private func makeMapper(json: String) throws -> GenreMapper {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("test_genres_\(UUID().uuidString).json")
        try json.write(to: url, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: url) }
        return try GenreMapper(jsonURL: url)
    }

    private let sampleJSON = """
    {
      "genres": [
        {
          "name": "Metal",
          "children": [
            {
              "name": "Thrash Metal",
              "children": [
                { "name": "Crossover Thrash", "children": [] }
              ]
            },
            { "name": "Death Metal", "children": [] },
            { "name": "Black Metal", "children": [] }
          ]
        },
        {
          "name": "Rock",
          "children": [
            { "name": "Progressive Rock", "children": [] },
            { "name": "Alternative Rock", "children": [] }
          ]
        }
      ]
    }
    """

    // MARK: - Tree Loading

    func testLoadTreeFromJSON() throws {
        let mapper = try makeMapper(json: sampleJSON)
        XCTAssertEqual(mapper.rootNodes.count, 2)
        XCTAssertEqual(mapper.rootNodes[0].name, "Metal")
        XCTAssertEqual(mapper.rootNodes[0].children?.count, 3)
        XCTAssertEqual(mapper.rootNodes[1].name, "Rock")
    }

    func testLeafNodesHaveNilChildren() throws {
        let mapper = try makeMapper(json: sampleJSON)
        let deathMetal = mapper.rootNodes[0].children?.first(where: { $0.name == "Death Metal" })
        XCTAssertNotNil(deathMetal)
        XCTAssertNil(deathMetal?.children)
    }

    // MARK: - Path-Based IDs

    func testPathBasedIDs() throws {
        let mapper = try makeMapper(json: sampleJSON)
        let metal = mapper.rootNodes[0]
        XCTAssertEqual(metal.id, "metal")
        let thrash = metal.children?.first(where: { $0.name == "Thrash Metal" })
        XCTAssertEqual(thrash?.id, "metal/thrash metal")
        let crossover = thrash?.children?.first(where: { $0.name == "Crossover Thrash" })
        XCTAssertEqual(crossover?.id, "metal/thrash metal/crossover thrash")
    }

    func testDuplicateGenreNamesHaveUniqueIDs() throws {
        // Genre "Funk" appearing under both "R&B" and "Pop"
        let json = """
        {
          "genres": [
            { "name": "R&B", "children": [{ "name": "Funk", "children": [] }] },
            { "name": "Pop", "children": [{ "name": "Funk", "children": [] }] }
          ]
        }
        """
        let mapper = try makeMapper(json: json)
        let rbFunk = mapper.rootNodes[0].children?.first
        let popFunk = mapper.rootNodes[1].children?.first
        XCTAssertEqual(rbFunk?.name, "Funk")
        XCTAssertEqual(popFunk?.name, "Funk")
        XCTAssertNotEqual(rbFunk?.id, popFunk?.id)
        XCTAssertEqual(rbFunk?.id, "r&b/funk")
        XCTAssertEqual(popFunk?.id, "pop/funk")
    }

    // MARK: - Exact Matching

    func testExactMatchCaseInsensitive() throws {
        let mapper = try makeMapper(json: sampleJSON)
        let node = mapper.matchGenre("thrash metal")
        XCTAssertNotNil(node)
        XCTAssertEqual(node?.name, "Thrash Metal")
    }

    func testExactMatchMixedCase() throws {
        let mapper = try makeMapper(json: sampleJSON)
        let node = mapper.matchGenre("PROGRESSIVE ROCK")
        XCTAssertEqual(node?.name, "Progressive Rock")
    }

    // MARK: - Fuzzy Matching

    func testFuzzyMatchCloseSpelling() throws {
        let mapper = try makeMapper(json: sampleJSON)
        let node = mapper.matchGenre("Progressve Rock")
        XCTAssertNotNil(node, "Fuzzy match should find 'Progressive Rock' for close spelling")
        XCTAssertEqual(node?.name, "Progressive Rock")
    }

    func testFuzzyMatchSlightTypo() throws {
        let mapper = try makeMapper(json: sampleJSON)
        let node = mapper.matchGenre("Deth Metal")
        XCTAssertNotNil(node)
        XCTAssertEqual(node?.name, "Death Metal")
    }

    func testNoMatchReturnsNil() throws {
        let mapper = try makeMapper(json: sampleJSON)
        let node = mapper.matchGenre("Baroque Chamber Opera")
        XCTAssertNil(node, "Completely unrelated genre should not match")
    }

    // MARK: - Display Tree & Pruning

    func testBuildDisplayTreePrunesEmptyBranches() throws {
        let mapper = try makeMapper(json: sampleJSON)
        let sets: [String: Set<Int64>] = ["Death Metal": [1, 2, 3, 4, 5], "Rock": [10, 11]]
        let tree = mapper.buildDisplayTree(genreAlbumSets: sets)

        let metal = tree.first(where: { $0.name == "Metal" })
        XCTAssertNotNil(metal, "Metal should survive because Death Metal has albums")
        let thrash = metal?.children?.first(where: { $0.name == "Thrash Metal" })
        XCTAssertNil(thrash, "Thrash Metal should be pruned — no albums")
        let death = metal?.children?.first(where: { $0.name == "Death Metal" })
        XCTAssertNotNil(death)
        XCTAssertEqual(death?.albumCount, 5)

        let rock = tree.first(where: { $0.name == "Rock" })
        XCTAssertNotNil(rock)
        XCTAssertEqual(rock?.albumCount, 2)
        XCTAssertNil(rock?.children, "Rock has no surviving children so children should be nil")
    }

    func testBuildDisplayTreeEmptySets() throws {
        let mapper = try makeMapper(json: sampleJSON)
        let tree = mapper.buildDisplayTree(genreAlbumSets: [:])
        XCTAssertTrue(tree.isEmpty, "Empty sets should produce empty tree")
    }

    // MARK: - Deduplicated Album Counts

    func testDeduplicatedCountWithSharedAlbum() throws {
        let mapper = try makeMapper(json: sampleJSON)
        // Album 1 is tagged with both Thrash Metal and Death Metal (siblings under Metal)
        let sets: [String: Set<Int64>] = [
            "Thrash Metal": [1, 2],
            "Death Metal": [1, 3],  // album 1 shared
        ]
        let tree = mapper.buildDisplayTree(genreAlbumSets: sets)
        let metal = tree.first(where: { $0.name == "Metal" })!

        // Metal's dedup count should be 3 (albums 1, 2, 3) not 4 (2 + 2)
        XCTAssertEqual(metal.deduplicatedTotalCount, 3)
        // Direct children still show their own counts
        let thrash = metal.children?.first(where: { $0.name == "Thrash Metal" })
        XCTAssertEqual(thrash?.albumCount, 2)
        XCTAssertEqual(thrash?.deduplicatedTotalCount, 2)
        let death = metal.children?.first(where: { $0.name == "Death Metal" })
        XCTAssertEqual(death?.albumCount, 2)
        XCTAssertEqual(death?.deduplicatedTotalCount, 2)
    }

    func testDeduplicatedCountParentAndChild() throws {
        let mapper = try makeMapper(json: sampleJSON)
        // Album 1 tagged both "Metal" and "Death Metal"
        let sets: [String: Set<Int64>] = [
            "Metal": [1],
            "Death Metal": [1, 2],
        ]
        let tree = mapper.buildDisplayTree(genreAlbumSets: sets)
        let metal = tree.first(where: { $0.name == "Metal" })!

        // Only 2 unique albums (1, 2)
        XCTAssertEqual(metal.deduplicatedTotalCount, 2)
    }

    // MARK: - Descendant Names

    func testAllDescendantNames() throws {
        let mapper = try makeMapper(json: sampleJSON)
        let metal = mapper.rootNodes.first(where: { $0.name == "Metal" })!
        let names = metal.allDescendantNames
        XCTAssertTrue(names.contains("Metal"))
        XCTAssertTrue(names.contains("Thrash Metal"))
        XCTAssertTrue(names.contains("Crossover Thrash"))
        XCTAssertTrue(names.contains("Death Metal"))
        XCTAssertTrue(names.contains("Black Metal"))
        XCTAssertEqual(names.count, 5)
    }

    // MARK: - Deduplication Scenario

    func testDeduplicationScenario() throws {
        let mapper = try makeMapper(json: sampleJSON)
        let metal = mapper.rootNodes.first(where: { $0.name == "Metal" })!
        let descendantNames = Set(metal.allDescendantNames)
        let albumTags = ["Metal", "Thrash Metal"]
        let matchingTags = albumTags.filter { descendantNames.contains($0) }
        XCTAssertEqual(matchingTags.count, 2, "Both tags should match")
    }
}
