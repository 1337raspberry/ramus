import XCTest
@testable import Cache

/// Tests for CacheDatabase's LIKE pattern escaping.
/// `escapeLikePattern` is private, so we test it indirectly by inserting data
/// with special characters and verifying search queries match correctly.
final class EscapeLikePatternTests: XCTestCase {

    var db: CacheDatabase!

    override func setUpWithError() throws {
        db = try CacheDatabase.temporary()
    }

    private func seedArtistAndAlbum(albumTitle: String, artistName: String = "Artist") throws -> (artistId: Int64, albumId: Int64) {
        let artistId = try db.upsertArtist(
            name: artistName, sortName: nil, sourceId: "a-\(UUID().uuidString)",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        let albumId = try db.upsertAlbum(
            title: albumTitle, artistId: artistId, year: 2020,
            sourceId: "al-\(UUID().uuidString)", artUrl: nil, updatedAt: nil
        )
        return (artistId, albumId)
    }

    // MARK: - Plain text

    func testPlainTextPassesThroughUnchanged() throws {
        let (_, _) = try seedArtistAndAlbum(albumTitle: "OK Computer")
        let results = try db.searchAlbumsByTitle(query: "OK Computer", albumIds: nil)
        XCTAssertEqual(results.count, 1)
        XCTAssertEqual(results.first?.albumTitle, "OK Computer")
    }

    // MARK: - Percent character

    func testPercentIsEscaped() throws {
        let (_, _) = try seedArtistAndAlbum(albumTitle: "100% Fun")
        // Search for "100%" — the % should be literal, not a wildcard
        let results = try db.searchAlbumsByTitle(query: "100%", albumIds: nil)
        XCTAssertEqual(results.count, 1)
        XCTAssertEqual(results.first?.albumTitle, "100% Fun")
    }

    func testPercentDoesNotMatchAsWildcard() throws {
        let (_, _) = try seedArtistAndAlbum(albumTitle: "Totally Different Album")
        // A bare "%" without escaping would match everything; with escaping it should not
        let results = try db.searchAlbumsByTitle(query: "%", albumIds: nil)
        XCTAssertTrue(results.isEmpty, "Literal % should not match album titles without %")
    }

    // MARK: - Underscore character

    func testUnderscoreIsEscaped() throws {
        let (_, _) = try seedArtistAndAlbum(albumTitle: "a_b test")
        let results = try db.searchAlbumsByTitle(query: "a_b", albumIds: nil)
        XCTAssertEqual(results.count, 1)
        XCTAssertEqual(results.first?.albumTitle, "a_b test")
    }

    func testUnderscoreDoesNotMatchAsSingleCharWildcard() throws {
        // Without escaping, "_" matches any single character
        let (_, _) = try seedArtistAndAlbum(albumTitle: "axb")
        let results = try db.searchAlbumsByTitle(query: "a_b", albumIds: nil)
        XCTAssertTrue(results.isEmpty, "Literal _ should not match 'axb' as a wildcard")
    }

    // MARK: - Backslash character

    func testBackslashIsEscaped() throws {
        let (_, _) = try seedArtistAndAlbum(albumTitle: "a\\b")
        let results = try db.searchAlbumsByTitle(query: "a\\b", albumIds: nil)
        XCTAssertEqual(results.count, 1)
        XCTAssertEqual(results.first?.albumTitle, "a\\b")
    }

    // MARK: - Combined special characters

    func testCombinedSpecialChars() throws {
        let (_, _) = try seedArtistAndAlbum(albumTitle: "100%_fun\\test")
        let results = try db.searchAlbumsByTitle(query: "100%_fun\\test", albumIds: nil)
        XCTAssertEqual(results.count, 1)
        XCTAssertEqual(results.first?.albumTitle, "100%_fun\\test")
    }

    // MARK: - Artist name search uses same escaping

    func testArtistSearchEscapesSpecialChars() throws {
        let (_, _) = try seedArtistAndAlbum(albumTitle: "Album", artistName: "100% Band")
        let results = try db.searchAlbumsByArtist(query: "100%", albumIds: nil)
        XCTAssertEqual(results.count, 1)
        XCTAssertEqual(results.first?.artistName, "100% Band")
    }
}
