import XCTest
@testable import Cache
import Models

final class AlbumYearRangeTests: XCTestCase {

    var db: CacheDatabase!

    override func setUpWithError() throws {
        db = try CacheDatabase.temporary()
        try seedAlbums()
    }

    private var albumIds: [Int: Int64] = [:]

    private func seedAlbums() throws {
        let artistId = try db.upsertArtist(
            name: "Artist", sortName: nil, sourceId: "a1",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        // Albums spanning several years
        albumIds[1986] = try db.upsertAlbum(
            title: "Reign in Blood", artistId: artistId, year: 1986,
            sourceId: "al1", artUrl: nil, updatedAt: nil
        )
        albumIds[1997] = try db.upsertAlbum(
            title: "OK Computer", artistId: artistId, year: 1997,
            sourceId: "al2", artUrl: nil, updatedAt: nil
        )
        albumIds[2000] = try db.upsertAlbum(
            title: "Kid A", artistId: artistId, year: 2000,
            sourceId: "al3", artUrl: nil, updatedAt: nil
        )
        albumIds[2007] = try db.upsertAlbum(
            title: "In Rainbows", artistId: artistId, year: 2007,
            sourceId: "al4", artUrl: nil, updatedAt: nil
        )
        // Album with nil year (no year column set)
        _ = try db.upsertAlbum(
            title: "No Year Album", artistId: artistId, year: nil,
            sourceId: "al5", artUrl: nil, updatedAt: nil
        )
    }

    // MARK: - Equal

    func testEqualReturnsExactYear() throws {
        let ids = try db.albumIdsForYearRange(op: .equal, value: 1997)
        XCTAssertEqual(ids.count, 1)
        XCTAssertTrue(ids.contains(albumIds[1997]!))
    }

    func testEqualNoMatch() throws {
        let ids = try db.albumIdsForYearRange(op: .equal, value: 2020)
        XCTAssertTrue(ids.isEmpty)
    }

    // MARK: - Greater Than

    func testGreaterThan() throws {
        let ids = try db.albumIdsForYearRange(op: .greaterThan, value: 1999)
        XCTAssertEqual(ids.count, 2)
        XCTAssertTrue(ids.contains(albumIds[2000]!))
        XCTAssertTrue(ids.contains(albumIds[2007]!))
        XCTAssertFalse(ids.contains(albumIds[1997]!))
    }

    // MARK: - Less Than

    func testLessThan() throws {
        let ids = try db.albumIdsForYearRange(op: .lessThan, value: 1997)
        XCTAssertEqual(ids.count, 1)
        XCTAssertTrue(ids.contains(albumIds[1986]!))
    }

    // MARK: - Greater Or Equal

    func testGreaterOrEqual() throws {
        let ids = try db.albumIdsForYearRange(op: .greaterOrEqual, value: 2000)
        XCTAssertEqual(ids.count, 2)
        XCTAssertTrue(ids.contains(albumIds[2000]!))
        XCTAssertTrue(ids.contains(albumIds[2007]!))
    }

    // MARK: - Less Or Equal

    func testLessOrEqual() throws {
        let ids = try db.albumIdsForYearRange(op: .lessOrEqual, value: 1997)
        XCTAssertEqual(ids.count, 2)
        XCTAssertTrue(ids.contains(albumIds[1986]!))
        XCTAssertTrue(ids.contains(albumIds[1997]!))
    }

    // MARK: - Null year excluded

    func testNullYearNotIncluded() throws {
        // Albums with NULL year should never match any range query
        let all = try db.albumIdsForYearRange(op: .greaterOrEqual, value: 0)
        // Should have 4 albums (1986, 1997, 2000, 2007) — not the nil-year album
        XCTAssertEqual(all.count, 4)
    }

    // MARK: - RangeOp.sqlLiteral coverage

    func testAllRangeOpSqlLiterals() {
        XCTAssertEqual(RangeOp.equal.sqlLiteral, "=")
        XCTAssertEqual(RangeOp.greaterThan.sqlLiteral, ">")
        XCTAssertEqual(RangeOp.lessThan.sqlLiteral, "<")
        XCTAssertEqual(RangeOp.greaterOrEqual.sqlLiteral, ">=")
        XCTAssertEqual(RangeOp.lessOrEqual.sqlLiteral, "<=")
    }
}
