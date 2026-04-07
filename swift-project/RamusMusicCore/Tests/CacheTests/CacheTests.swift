import XCTest
@testable import Cache

final class CacheDatabaseTests: XCTestCase {

    var db: CacheDatabase!

    override func setUpWithError() throws {
        db = try CacheDatabase.temporary()
    }

    // MARK: - Artist CRUD

    func testUpsertArtist() throws {
        let id = try db.upsertArtist(
            name: "Radiohead", sortName: "Radiohead", sourceId: "100",
            artUrl: nil, summary: "A band", updatedAt: 1000
        )
        XCTAssertGreaterThan(id, 0)

        // Upsert same sourceId updates, returns same id
        let id2 = try db.upsertArtist(
            name: "Radiohead (Updated)", sortName: "Radiohead", sourceId: "100",
            artUrl: "/thumb.jpg", summary: nil, updatedAt: 2000
        )
        XCTAssertEqual(id, id2)
    }

    // MARK: - Album CRUD

    func testUpsertAlbum() throws {
        let artistId = try db.upsertArtist(
            name: "Artist", sortName: nil, sourceId: "a1",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        let albumId = try db.upsertAlbum(
            title: "OK Computer", artistId: artistId, year: 1997,
            sourceId: "al1", artUrl: nil, updatedAt: nil
        )
        XCTAssertGreaterThan(albumId, 0)
    }

    // MARK: - Track CRUD

    func testUpsertTrack() throws {
        let artistId = try db.upsertArtist(
            name: "Artist", sortName: nil, sourceId: "a1",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        let albumId = try db.upsertAlbum(
            title: "Album", artistId: artistId, year: 2020,
            sourceId: "al1", artUrl: nil, updatedAt: nil
        )
        let trackId = try db.upsertTrack(
            title: "Song", albumId: albumId, artistId: artistId,
            trackNumber: 1, discNumber: 1, durationMs: 240000,
            sourceId: "t1", codec: "flac",
            partKey: "/library/parts/1", streamId: nil, updatedAt: nil
        )
        XCTAssertGreaterThan(trackId, 0)

        let tracks = try db.tracksForAlbum(sourceId: "al1")
        XCTAssertEqual(tracks.count, 1)
        XCTAssertEqual(tracks.first?.title, "Song")
    }

    // MARK: - Genres + album_genres

    func testMultipleGenresPerAlbum() throws {
        let artistId = try db.upsertArtist(
            name: "Artist", sortName: nil, sourceId: "a1",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        let albumId = try db.upsertAlbum(
            title: "Album", artistId: artistId, year: 2020,
            sourceId: "al1", artUrl: nil, updatedAt: nil
        )

        let rockId = try db.upsertGenre(name: "Rock")
        let progId = try db.upsertGenre(name: "Progressive Rock")
        let artId = try db.upsertGenre(name: "Art Rock")

        try db.setAlbumGenres(albumId: albumId, genreIds: [rockId, progId, artId])

        // Album should appear in all three genre queries
        let rockAlbums = try db.albumsByGenreIds([rockId])
        let progAlbums = try db.albumsByGenreIds([progId])
        let artAlbums = try db.albumsByGenreIds([artId])

        XCTAssertEqual(rockAlbums.count, 1)
        XCTAssertEqual(progAlbums.count, 1)
        XCTAssertEqual(artAlbums.count, 1)
        XCTAssertEqual(rockAlbums.first?.title, "Album")
    }

    func testGenreUpsertIsCaseInsensitive() throws {
        let id1 = try db.upsertGenre(name: "Rock")
        let id2 = try db.upsertGenre(name: "rock")
        let id3 = try db.upsertGenre(name: "ROCK")

        XCTAssertEqual(id1, id2)
        XCTAssertEqual(id2, id3)
    }

    func testSetAlbumGenresReplacesExisting() throws {
        let artistId = try db.upsertArtist(
            name: "A", sortName: nil, sourceId: "a1",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        let albumId = try db.upsertAlbum(
            title: "Al", artistId: artistId, year: nil,
            sourceId: "al1", artUrl: nil, updatedAt: nil
        )

        let g1 = try db.upsertGenre(name: "Pop")
        let g2 = try db.upsertGenre(name: "Rock")
        let g3 = try db.upsertGenre(name: "Jazz")

        // Set initial genres
        try db.setAlbumGenres(albumId: albumId, genreIds: [g1, g2])
        var popAlbums = try db.albumsByGenreIds([g1])
        XCTAssertEqual(popAlbums.count, 1)

        // Replace with different genres
        try db.setAlbumGenres(albumId: albumId, genreIds: [g3])
        popAlbums = try db.albumsByGenreIds([g1])
        XCTAssertEqual(popAlbums.count, 0, "Old genre link should be removed")

        let jazzAlbums = try db.albumsByGenreIds([g3])
        XCTAssertEqual(jazzAlbums.count, 1)
    }

    // MARK: - Batch Upserts

    func testBatchUpsertArtists() throws {
        let items: [(name: String, sortName: String?, sourceId: String,
                      artUrl: String?, summary: String?, updatedAt: Int?)] = [
            (name: "Radiohead", sortName: "Radiohead", sourceId: "a1", artUrl: nil, summary: nil, updatedAt: 1000),
            (name: "Björk", sortName: "Bjork", sourceId: "a2", artUrl: nil, summary: nil, updatedAt: 2000),
            (name: "Aphex Twin", sortName: "Aphex Twin", sourceId: "a3", artUrl: nil, summary: nil, updatedAt: 3000),
        ]
        let map = try db.batchUpsertArtists(items)

        XCTAssertEqual(map.count, 3)
        XCTAssertNotNil(map["a1"])
        XCTAssertNotNil(map["a2"])
        XCTAssertNotNil(map["a3"])

        // Upsert again with updated name — should return same IDs
        let updated: [(name: String, sortName: String?, sourceId: String,
                        artUrl: String?, summary: String?, updatedAt: Int?)] = [
            (name: "Radiohead (Updated)", sortName: "Radiohead", sourceId: "a1", artUrl: nil, summary: nil, updatedAt: 4000),
        ]
        let map2 = try db.batchUpsertArtists(updated)
        XCTAssertEqual(map["a1"], map2["a1"])
    }

    func testBatchUpsertAlbums() throws {
        let artistId = try db.upsertArtist(
            name: "Artist", sortName: nil, sourceId: "a1",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        let items: [(title: String, artistId: Int64, year: Int?,
                      sourceId: String, artUrl: String?, rating: Double?,
                      studio: String?, updatedAt: Int?,
                      addedAt: Int?, lastViewedAt: Int?)] = [
            (title: "Album 1", artistId: artistId, year: 2000, sourceId: "al1", artUrl: nil, rating: nil, studio: nil, updatedAt: 100, addedAt: nil, lastViewedAt: nil),
            (title: "Album 2", artistId: artistId, year: 2005, sourceId: "al2", artUrl: nil, rating: nil, studio: nil, updatedAt: 200, addedAt: nil, lastViewedAt: nil),
        ]
        let map = try db.batchUpsertAlbums(items)
        XCTAssertEqual(map.count, 2)
        XCTAssertNotNil(map["al1"])
        XCTAssertNotNil(map["al2"])
        XCTAssertNotEqual(map["al1"], map["al2"])
    }

    func testBatchUpsertTracks() throws {
        let artistId = try db.upsertArtist(
            name: "A", sortName: nil, sourceId: "a1",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        let albumId = try db.upsertAlbum(
            title: "Al", artistId: artistId, year: nil,
            sourceId: "al1", artUrl: nil, updatedAt: nil
        )
        let items: [(title: String, albumId: Int64, artistId: Int64,
                      trackNumber: Int?, discNumber: Int?, durationMs: Int?,
                      sourceId: String, codec: String?, partKey: String?,
                      streamId: Int?, userRating: Double?, bitrate: Int?,
                      trackArtist: String?, updatedAt: Int?)] = [
            (title: "Track 1", albumId: albumId, artistId: artistId, trackNumber: 1, discNumber: 1, durationMs: 200000, sourceId: "t1", codec: "flac", partKey: nil, streamId: nil, userRating: nil, bitrate: nil, trackArtist: nil, updatedAt: 100),
            (title: "Track 2", albumId: albumId, artistId: artistId, trackNumber: 2, discNumber: 1, durationMs: 180000, sourceId: "t2", codec: "flac", partKey: nil, streamId: nil, userRating: nil, bitrate: nil, trackArtist: nil, updatedAt: 100),
            (title: "Track 3", albumId: albumId, artistId: artistId, trackNumber: 3, discNumber: 1, durationMs: 220000, sourceId: "t3", codec: "flac", partKey: nil, streamId: nil, userRating: nil, bitrate: nil, trackArtist: nil, updatedAt: 100),
        ]
        try db.batchUpsertTracks(items)

        let tracks = try db.tracksForAlbum(sourceId: "al1")
        XCTAssertEqual(tracks.count, 3)
    }

    func testBatchUpsertGenresAndLinks() throws {
        let artistId = try db.upsertArtist(
            name: "A", sortName: nil, sourceId: "a1",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        let albumId = try db.upsertAlbum(
            title: "Al", artistId: artistId, year: nil,
            sourceId: "al1", artUrl: nil, updatedAt: nil
        )
        try db.batchUpsertGenresAndLinks([
            (albumId: albumId, genreName: "Rock"),
            (albumId: albumId, genreName: "Metal"),
        ])

        let rockId = try db.upsertGenre(name: "Rock") // should return existing
        let metalId = try db.upsertGenre(name: "Metal")
        let albums = try db.albumsByGenreIds([rockId, metalId])
        XCTAssertEqual(albums.count, 1)
    }

    // MARK: - Bulk Timestamp Lookups

    func testAllArtistTimestamps() throws {
        try db.upsertArtist(name: "A", sortName: nil, sourceId: "a1", artUrl: nil, summary: nil, updatedAt: 1000)
        try db.upsertArtist(name: "B", sortName: nil, sourceId: "a2", artUrl: nil, summary: nil, updatedAt: 2000)
        try db.upsertArtist(name: "C", sortName: nil, sourceId: "a3", artUrl: nil, summary: nil, updatedAt: nil)

        let timestamps = try db.allArtistTimestamps()
        XCTAssertEqual(timestamps.count, 3)
        XCTAssertEqual(timestamps["a1"]?.updatedAt, 1000)
        XCTAssertEqual(timestamps["a2"]?.updatedAt, 2000)
        XCTAssertNil(timestamps["a3"]?.updatedAt)
        XCTAssertGreaterThan(timestamps["a1"]!.id, 0)
    }

    func testAllAlbumTimestamps() throws {
        let artistId = try db.upsertArtist(name: "A", sortName: nil, sourceId: "a1", artUrl: nil, summary: nil, updatedAt: nil)
        try db.upsertAlbum(title: "Al1", artistId: artistId, year: nil, sourceId: "al1", artUrl: nil, updatedAt: 500)
        try db.upsertAlbum(title: "Al2", artistId: artistId, year: nil, sourceId: "al2", artUrl: nil, updatedAt: 600)

        let timestamps = try db.allAlbumTimestamps()
        XCTAssertEqual(timestamps.count, 2)
        XCTAssertEqual(timestamps["al1"]?.updatedAt, 500)
        XCTAssertEqual(timestamps["al2"]?.updatedAt, 600)
    }

    // MARK: - FTS5 Search

    func testFTS5PrefixSearch() throws {
        let artistId = try db.upsertArtist(
            name: "A", sortName: nil, sourceId: "a1",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        let albumId = try db.upsertAlbum(
            title: "Al", artistId: artistId, year: nil,
            sourceId: "al1", artUrl: nil, updatedAt: nil
        )

        try db.upsertTrack(
            title: "Paranoid Android", albumId: albumId, artistId: artistId,
            trackNumber: 1, discNumber: 1, durationMs: 300000,
            sourceId: "t1", codec: "flac",
            partKey: nil, streamId: nil, updatedAt: nil
        )
        try db.upsertTrack(
            title: "Karma Police", albumId: albumId, artistId: artistId,
            trackNumber: 2, discNumber: 1, durationMs: 260000,
            sourceId: "t2", codec: "flac",
            partKey: nil, streamId: nil, updatedAt: nil
        )
        try db.upsertTrack(
            title: "Lucky", albumId: albumId, artistId: artistId,
            trackNumber: 3, discNumber: 1, durationMs: 270000,
            sourceId: "t3", codec: "flac",
            partKey: nil, streamId: nil, updatedAt: nil
        )

        // Prefix search "par" should match "Paranoid Android"
        let results = try db.searchTracksEnriched(ftsQuery: "\"par\"*", albumIds: nil)
        XCTAssertEqual(results.count, 1)
        XCTAssertEqual(results.first?.trackTitle, "Paranoid Android")

        // Prefix search "ka" should match "Karma Police"
        let results2 = try db.searchTracksEnriched(ftsQuery: "\"ka\"*", albumIds: nil)
        XCTAssertEqual(results2.count, 1)
        XCTAssertEqual(results2.first?.trackTitle, "Karma Police")

        // Search "lu" should match "Lucky"
        let results3 = try db.searchTracksEnriched(ftsQuery: "\"lu\"*", albumIds: nil)
        XCTAssertEqual(results3.count, 1)
    }

    // MARK: - Stats

    func testStats() throws {
        let stats = try db.stats()
        XCTAssertEqual(stats.artistCount, 0)

        let artistId = try db.upsertArtist(
            name: "A", sortName: nil, sourceId: "a1",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        let albumId = try db.upsertAlbum(
            title: "Al", artistId: artistId, year: nil,
            sourceId: "al1", artUrl: nil, updatedAt: nil
        )
        try db.upsertTrack(
            title: "T", albumId: albumId, artistId: artistId,
            trackNumber: 1, discNumber: 1, durationMs: 100,
            sourceId: "t1", codec: nil,
            partKey: nil, streamId: nil, updatedAt: nil
        )
        _ = try db.upsertGenre(name: "Rock")

        let stats2 = try db.stats()
        XCTAssertEqual(stats2.artistCount, 1)
        XCTAssertEqual(stats2.albumCount, 1)
        XCTAssertEqual(stats2.trackCount, 1)
        XCTAssertEqual(stats2.genreCount, 1)
    }
}
