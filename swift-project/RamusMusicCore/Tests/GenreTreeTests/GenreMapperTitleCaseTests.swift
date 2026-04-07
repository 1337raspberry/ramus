import XCTest
@testable import GenreTree

final class GenreMapperTitleCaseTests: XCTestCase {

    // titleCase is private, so we test it indirectly through GenreMapper's
    // public API — genre names pass through titleCase during JSON loading.

    private func makeMapper(names: [String]) throws -> GenreMapper {
        let genres = names.map { "{\"name\":\"\($0)\",\"children\":[]}" }.joined(separator: ",")
        let json = "{\"genres\":[\(genres)]}"
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("titlecase_\(UUID().uuidString).json")
        try json.write(to: url, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: url) }
        return try GenreMapper(jsonURL: url)
    }

    func testAllLowercaseGetsTitleCased() throws {
        let mapper = try makeMapper(names: ["ambient music"])
        XCTAssertEqual(mapper.rootNodes.first?.name, "Ambient Music")
    }

    func testAcronymLeftAlone() throws {
        let mapper = try makeMapper(names: ["EBM"])
        XCTAssertEqual(mapper.rootNodes.first?.name, "EBM")
    }

    func testAmpersandAcronymLeftAlone() throws {
        let mapper = try makeMapper(names: ["R&B"])
        XCTAssertEqual(mapper.rootNodes.first?.name, "R&B")
    }

    func testHyphenatedCompound() throws {
        let mapper = try makeMapper(names: ["lo-fi"])
        XCTAssertEqual(mapper.rootNodes.first?.name, "Lo-Fi")
    }

    func testMixedCasePerWordPreservation() throws {
        // titleCase operates per-word (space-separated), then per-segment (hyphen-separated).
        // "death" is all lowercase → "Death". "Metal" has uppercase → left as "Metal".
        let mapper = try makeMapper(names: ["death Metal"])
        XCTAssertEqual(mapper.rootNodes.first?.name, "Death Metal")
    }

    func testWordWithExistingUppercaseLeftAlone() throws {
        // "dEath" contains uppercase → left untouched
        let mapper = try makeMapper(names: ["dEath metal"])
        XCTAssertEqual(mapper.rootNodes.first?.name, "dEath Metal")
    }

    func testSingleLowercaseWord() throws {
        let mapper = try makeMapper(names: ["jazz"])
        XCTAssertEqual(mapper.rootNodes.first?.name, "Jazz")
    }

    func testEmptyStringReturnsEmpty() throws {
        let mapper = try makeMapper(names: [""])
        XCTAssertEqual(mapper.rootNodes.first?.name, "")
    }

    func testMultipleHyphenSegments() throws {
        // "drum-and-bass" — all segments lowercase, all get capitalised
        let mapper = try makeMapper(names: ["drum-and-bass"])
        XCTAssertEqual(mapper.rootNodes.first?.name, "Drum-And-Bass")
    }

    func testMixedHyphenSegments() throws {
        // "lo-FI" — "lo" is lowercase → "Lo", "FI" has uppercase → "FI"
        let mapper = try makeMapper(names: ["lo-FI"])
        XCTAssertEqual(mapper.rootNodes.first?.name, "Lo-FI")
    }
}
