import XCTest
@testable import GenreTree

final class CustomGenreParserTests: XCTestCase {

    // MARK: - Happy Path

    func testBasicHierarchy() throws {
        let text = """
        Rock
          Alternative Rock
            Shoegaze
          Punk Rock
        Electronic
          Ambient
        """
        let (data, warnings) = try CustomGenreParser.parse(text)
        XCTAssertTrue(warnings.isEmpty)

        let mapper = try makeMapper(data: data)
        XCTAssertEqual(mapper.rootNodes.count, 2)
        XCTAssertEqual(mapper.rootNodes[0].name, "Rock")
        XCTAssertEqual(mapper.rootNodes[0].children?.count, 2)
        XCTAssertEqual(mapper.rootNodes[0].children?[0].name, "Alternative Rock")
        XCTAssertEqual(mapper.rootNodes[0].children?[0].children?.count, 1)
        XCTAssertEqual(mapper.rootNodes[0].children?[0].children?[0].name, "Shoegaze")
        XCTAssertEqual(mapper.rootNodes[0].children?[1].name, "Punk Rock")
        XCTAssertEqual(mapper.rootNodes[1].name, "Electronic")
        XCTAssertEqual(mapper.rootNodes[1].children?.count, 1)
    }

    func testTabIndentation() throws {
        let text = "Rock\n\tAlternative Rock\n\t\tShoegaze\n\tPunk Rock"
        let (data, warnings) = try CustomGenreParser.parse(text)
        XCTAssertTrue(warnings.isEmpty)

        let mapper = try makeMapper(data: data)
        XCTAssertEqual(mapper.rootNodes.count, 1)
        XCTAssertEqual(mapper.rootNodes[0].children?.count, 2)
        XCTAssertEqual(mapper.rootNodes[0].children?[0].children?[0].name, "Shoegaze")
    }

    func testFourSpaceIndentation() throws {
        let text = "Rock\n    Alternative Rock\n        Shoegaze"
        let (data, warnings) = try CustomGenreParser.parse(text)
        XCTAssertTrue(warnings.isEmpty)

        let mapper = try makeMapper(data: data)
        XCTAssertEqual(mapper.rootNodes[0].children?[0].children?[0].name, "Shoegaze")
    }

    // MARK: - Descriptions

    func testOptionalDescriptions() throws {
        let text = """
        Rock[Guitar-driven music]
          Shoegaze[Wall of sound with ethereal vocals]
          Punk Rock
        Jazz
        """
        let (data, warnings) = try CustomGenreParser.parse(text)
        XCTAssertTrue(warnings.isEmpty)

        let mapper = try makeMapper(data: data)
        XCTAssertEqual(mapper.rootNodes[0].shortSummary, "Guitar-driven music")
        XCTAssertEqual(mapper.rootNodes[0].children?[0].shortSummary, "Wall of sound with ethereal vocals")
        XCTAssertNil(mapper.rootNodes[0].children?[1].shortSummary)
        XCTAssertNil(mapper.rootNodes[1].shortSummary)
    }

    func testEmptyBracketsNoDescription() throws {
        let text = "Rock[]\n  Punk Rock"
        let (data, _) = try CustomGenreParser.parse(text)
        let mapper = try makeMapper(data: data)
        XCTAssertNil(mapper.rootNodes[0].shortSummary)
    }

    // MARK: - Validation Errors

    func testEmptyFile() {
        XCTAssertThrowsError(try CustomGenreParser.parse("")) { error in
            XCTAssertEqual(error as? CustomGenreParseError, .emptyFile)
        }
    }

    func testWhitespaceOnlyFile() {
        XCTAssertThrowsError(try CustomGenreParser.parse("   \n\n  \n")) { error in
            XCTAssertEqual(error as? CustomGenreParseError, .emptyFile)
        }
    }

    func testFileTooLarge() {
        let big = String(repeating: "A", count: CustomGenreParser.maxFileSize + 1)
        XCTAssertThrowsError(try CustomGenreParser.parse(big)) { error in
            guard case .fileTooLarge = error as? CustomGenreParseError else {
                XCTFail("Expected fileTooLarge, got \(error)")
                return
            }
        }
    }

    func testTooManyLines() {
        // 50,001 lines — each line is a root genre
        let lines = (1...50_001).map { "Genre\($0)" }
        let text = lines.joined(separator: "\n")
        XCTAssertThrowsError(try CustomGenreParser.parse(text)) { error in
            guard case .tooManyLines = error as? CustomGenreParseError else {
                XCTFail("Expected tooManyLines, got \(error)")
                return
            }
        }
    }

    func testIndentationJump() {
        let text = "Rock\n        Deep Nested"  // 8 spaces with 2-space unit = level 4 jump
        // Actually with our detection, first indented line has 8 spaces → 4-space unit → depth 2
        // That's a jump from 0 to 2
        XCTAssertThrowsError(try CustomGenreParser.parse(text)) { error in
            guard case .indentationJump = error as? CustomGenreParseError else {
                XCTFail("Expected indentationJump, got \(error)")
                return
            }
        }
    }

    func testUnmatchedBracket() {
        let text = "Rock[missing close bracket"
        XCTAssertThrowsError(try CustomGenreParser.parse(text)) { error in
            guard case .unmatchedBracket = error as? CustomGenreParseError else {
                XCTFail("Expected unmatchedBracket, got \(error)")
                return
            }
        }
    }

    func testNameTooLong() {
        let longName = String(repeating: "A", count: 201)
        XCTAssertThrowsError(try CustomGenreParser.parse(longName)) { error in
            guard case .nameTooLong = error as? CustomGenreParseError else {
                XCTFail("Expected nameTooLong, got \(error)")
                return
            }
        }
    }

    func testJSONInputRejected() {
        let json = """
        {
          "genres": [
            { "name": "Rock" }
          ]
        }
        """
        XCTAssertThrowsError(try CustomGenreParser.parse(json)) { error in
            XCTAssertEqual(error as? CustomGenreParseError, .notPlainText)
        }
    }

    func testJSONArrayInputRejected() {
        let json = "[{\"name\": \"Rock\"}]"
        XCTAssertThrowsError(try CustomGenreParser.parse(json)) { error in
            XCTAssertEqual(error as? CustomGenreParseError, .notPlainText)
        }
    }

    func testNoRootGenres() {
        let text = "  Indented Only\n  Another Indented"
        XCTAssertThrowsError(try CustomGenreParser.parse(text)) { error in
            XCTAssertEqual(error as? CustomGenreParseError, .noRootGenresFound)
        }
    }

    // MARK: - Warnings

    func testDuplicateNameWarning() throws {
        let text = "Rock\nJazz\nRock"  // duplicate root "Rock"
        let (_, warnings) = try CustomGenreParser.parse(text)
        XCTAssertEqual(warnings.count, 1)
        XCTAssertTrue(warnings[0].contains("duplicate"))
        XCTAssertTrue(warnings[0].contains("Rock"))
    }

    func testDuplicateCaseInsensitive() throws {
        let text = "Rock\nrock"
        let (_, warnings) = try CustomGenreParser.parse(text)
        XCTAssertEqual(warnings.count, 1)
    }

    func testDuplicatesAllowedAcrossParents() throws {
        let text = "Electronic\n  Funk\nR&B\n  Funk"
        let (_, warnings) = try CustomGenreParser.parse(text)
        XCTAssertTrue(warnings.isEmpty, "Same name under different parents should not warn")
    }

    // MARK: - Edge Cases

    func testBlankLinesIgnored() throws {
        let text = "Rock\n\n  Shoegaze\n\n\nJazz"
        let (data, _) = try CustomGenreParser.parse(text)
        let mapper = try makeMapper(data: data)
        XCTAssertEqual(mapper.rootNodes.count, 2)
    }

    func testC0ControlCharactersStripped() throws {
        let text = "Rock\u{00}\u{01}\n  Shoegaze"
        let (data, _) = try CustomGenreParser.parse(text)
        let mapper = try makeMapper(data: data)
        XCTAssertEqual(mapper.rootNodes[0].name, "Rock")
    }

    func testC1ControlCharactersStripped() throws {
        // U+0080 (PAD), U+008B (PARTIAL LINE FORWARD), U+0090 (DCS)
        let text = "Ro\u{0080}ck\u{008B}\u{0090}\n  Shoe\u{009F}gaze"
        let (data, _) = try CustomGenreParser.parse(text)
        let mapper = try makeMapper(data: data)
        XCTAssertEqual(mapper.rootNodes[0].name, "Rock")
        XCTAssertEqual(mapper.rootNodes[0].children?[0].name, "Shoegaze")
    }

    func testUnicodePreserved() throws {
        let text = "Música Brasileira\n  Bossa Nova\nJazz café"
        let (data, _) = try CustomGenreParser.parse(text)
        let mapper = try makeMapper(data: data)
        XCTAssertEqual(mapper.rootNodes[0].name, "Música Brasileira")
        XCTAssertEqual(mapper.rootNodes[1].name, "Jazz Café")
    }

    func testMixedTabsAndSpacesUsesDetectedUnit() throws {
        // First indented line uses tab → indent unit locks to tab.
        // Subsequent space-indented lines get depth 0 (treated as roots).
        let text = "Rock\n\tAlternative Rock\n  Punk Rock"
        let (data, _) = try CustomGenreParser.parse(text)
        let mapper = try makeMapper(data: data)
        // "Alternative Rock" is depth 1 (tab), "Punk Rock" is depth 0 (spaces → 0 tabs)
        XCTAssertEqual(mapper.rootNodes.count, 2, "Space-indented line becomes root when unit is tabs")
        XCTAssertEqual(mapper.rootNodes[0].name, "Rock")
        XCTAssertEqual(mapper.rootNodes[0].children?.count, 1)
        XCTAssertEqual(mapper.rootNodes[1].name, "Punk Rock")
    }

    func testLeafNodesHaveNilChildren() throws {
        let text = "Rock\n  Shoegaze"
        let (data, _) = try CustomGenreParser.parse(text)
        let mapper = try makeMapper(data: data)
        let shoegaze = mapper.rootNodes[0].children?[0]
        XCTAssertNil(shoegaze?.children)
    }

    func testSingleRootGenre() throws {
        let text = "Rock"
        let (data, _) = try CustomGenreParser.parse(text)
        let mapper = try makeMapper(data: data)
        XCTAssertEqual(mapper.rootNodes.count, 1)
        XCTAssertEqual(mapper.rootNodes[0].name, "Rock")
        XCTAssertNil(mapper.rootNodes[0].children)
    }

    func testDeeplyNested() throws {
        var text = "Level0"
        for i in 1...8 {
            text += "\n" + String(repeating: "  ", count: i) + "Level\(i)"
        }
        let (data, _) = try CustomGenreParser.parse(text)
        let mapper = try makeMapper(data: data)
        var node = mapper.rootNodes[0]
        for i in 1...8 {
            XCTAssertEqual(node.children?.count, 1)
            node = node.children![0]
            XCTAssertEqual(node.name, "Level\(i)")
        }
    }

    func testDescriptionWithBracketsInside() throws {
        // Genre name ends at first [, description goes to last ]
        let text = "Rock[includes [sub]genres]"
        let (data, _) = try CustomGenreParser.parse(text)
        let mapper = try makeMapper(data: data)
        XCTAssertEqual(mapper.rootNodes[0].name, "Rock")
        XCTAssertEqual(mapper.rootNodes[0].shortSummary, "includes [sub]genres")
    }

    func testDescriptionOnlyLineSkipped() throws {
        let text = "Rock\n  [just a description]\n  Punk Rock"
        let (data, warnings) = try CustomGenreParser.parse(text)
        XCTAssertEqual(warnings.count, 1)
        XCTAssertTrue(warnings[0].contains("no genre name"))
        let mapper = try makeMapper(data: data)
        XCTAssertEqual(mapper.rootNodes[0].children?.count, 1)
        XCTAssertEqual(mapper.rootNodes[0].children?[0].name, "Punk Rock")
    }

    func testSameChildNameUnderDifferentParentsNoWarning() throws {
        let text = "Rock\n  Alternative Rock\n  Punk Rock\nJazz\n  Alternative Rock"
        let (_, warnings) = try CustomGenreParser.parse(text)
        XCTAssertTrue(warnings.isEmpty, "Same child under different parents should not warn")
    }

    func testRootDuplicateStillDetected() throws {
        let text = "Rock\n  Punk\nJazz\nRock"
        let (_, warnings) = try CustomGenreParser.parse(text)
        XCTAssertEqual(warnings.count, 1)
        XCTAssertTrue(warnings[0].contains("Rock"))
    }

    func testRoundTripThroughGenreMapper() throws {
        let text = """
        Metal[Heavy guitar music]
          Thrash Metal[Fast and aggressive]
            Crossover Thrash
          Death Metal
          Black Metal
        Rock
          Progressive Rock
        """
        let (data, _) = try CustomGenreParser.parse(text)
        let mapper = try makeMapper(data: data)

        // Verify the mapper can match genres
        XCTAssertNotNil(mapper.matchGenre("thrash metal"))
        XCTAssertNotNil(mapper.matchGenre("Progressive Rock"))
        XCTAssertEqual(mapper.matchGenre("crossover thrash")?.name, "Crossover Thrash")

        // Verify display tree works
        let sets: [String: Set<Int64>] = [
            "Death Metal": [1, 2],
            "Rock": [3],
        ]
        let tree = mapper.buildDisplayTree(genreAlbumSets: sets)
        XCTAssertFalse(tree.isEmpty)
    }

    // MARK: - Helpers

    private func makeMapper(data: Data) throws -> GenreMapper {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("test_custom_\(UUID().uuidString).json")
        try data.write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }
        return try GenreMapper(jsonURL: url)
    }
}

// MARK: - Equatable conformance for error matching

extension CustomGenreParseError: Equatable {
    public static func == (lhs: CustomGenreParseError, rhs: CustomGenreParseError) -> Bool {
        switch (lhs, rhs) {
        case (.emptyFile, .emptyFile),
             (.noRootGenresFound, .noRootGenresFound),
             (.notPlainText, .notPlainText):
            true
        case (.fileTooLarge(let a), .fileTooLarge(let b)):
            a == b
        case (.tooManyLines(let a), .tooManyLines(let b)):
            a == b
        case (.nameTooLong(let la, _), .nameTooLong(let lb, _)):
            la == lb
        case (.unmatchedBracket(let a), .unmatchedBracket(let b)):
            a == b
        case (.indentationJump(let la, _, _), .indentationJump(let lb, _, _)):
            la == lb
        default:
            false
        }
    }
}
