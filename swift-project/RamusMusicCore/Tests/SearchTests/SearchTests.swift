import XCTest
@testable import Search
@testable import Cache
@testable import GenreTree

// MARK: - QueryParser Tests

final class QueryParserTests: XCTestCase {

    func testParseFreeText() {
        let q = QueryParser.parse("hello world")
        XCTAssertEqual(q.filters.count, 1)
        XCTAssertEqual(q.freeText, "hello world")
    }

    func testParseGenreFilter() {
        let q = QueryParser.parse("/metal")
        XCTAssertEqual(q.genreFilters, ["metal"])
    }

    func testParseArtistFilter() {
        let q = QueryParser.parse("@metallica")
        XCTAssertEqual(q.artistFilters, ["metallica"])
    }

    func testParseAlbumTitleFilter() {
        let q = QueryParser.parse("%reign")
        XCTAssertEqual(q.albumTitleFilters, ["reign"])
    }

    func testParseTrackSearch() {
        let q = QueryParser.parse("!paranoid")
        XCTAssertEqual(q.trackSearches, ["paranoid"])
        XCTAssertTrue(q.hasTrackSearch)
    }

    func testParseMultiWordTrackSearch() {
        let q = QueryParser.parse("!Baby Blue")
        XCTAssertEqual(q.trackSearches, ["Baby Blue"])
        XCTAssertNil(q.freeText)
    }

    func testParseMultiWordAlbumTitleSearch() {
        let q = QueryParser.parse("%OK Computer")
        XCTAssertEqual(q.albumTitleFilters, ["OK Computer"])
        XCTAssertNil(q.freeText)
    }

    func testParseYearGreaterThan() {
        let q = QueryParser.parse("year:>2000")
        let r = q.rangeFilters.first!
        XCTAssertEqual(r.0, .year)
        XCTAssertEqual(r.1, .greaterThan)
        XCTAssertEqual(r.2, 2000)
    }

    func testParseYearEqual() {
        let q = QueryParser.parse("year:1997")
        let r = q.rangeFilters.first!
        XCTAssertEqual(r.1, .equal)
        XCTAssertEqual(r.2, 1997)
    }

    func testParseRatingGreaterOrEqual() {
        let q = QueryParser.parse("rating:>=8")
        let r = q.rangeFilters.first!
        XCTAssertEqual(r.0, .rating)
        XCTAssertEqual(r.1, .greaterOrEqual)
        XCTAssertEqual(r.2, 8)
    }

    func testParseLessThanOrEqual() {
        let q = QueryParser.parse("year:<=1990")
        let r = q.rangeFilters.first!
        XCTAssertEqual(r.1, .lessOrEqual)
        XCTAssertEqual(r.2, 1990)
    }

    func testParseCombinedWithAND() {
        let q = QueryParser.parse("/metal AND @slayer AND year:>1985 AND !reign")
        XCTAssertEqual(q.genreFilters, ["metal"])
        XCTAssertEqual(q.artistFilters, ["slayer"])
        XCTAssertEqual(q.rangeFilters.first?.2, 1985)
        XCTAssertEqual(q.trackSearches, ["reign"])
    }

    func testMultiWordGenreWithoutAND() {
        let q = QueryParser.parse("/post rock")
        XCTAssertEqual(q.genreFilters, ["post rock"])
        XCTAssertNil(q.freeText)
    }

    func testMultiWordArtistWithoutAND() {
        let q = QueryParser.parse("@blue öyster cult")
        XCTAssertEqual(q.artistFilters, ["blue öyster cult"])
        XCTAssertNil(q.freeText)
    }

    func testOperatorWithoutANDConsumesAll() {
        // Without AND, the entire input belongs to the first operator
        let q = QueryParser.parse("/rock year:2000")
        XCTAssertEqual(q.genreFilters, ["rock year:2000"])
        XCTAssertTrue(q.rangeFilters.isEmpty)
    }

    func testParseEmptyInput() {
        let q = QueryParser.parse("")
        XCTAssertTrue(q.isEmpty)
    }

    func testParseInvalidYear() {
        let q = QueryParser.parse("year:abc")
        XCTAssertTrue(q.rangeFilters.isEmpty)
        XCTAssertEqual(q.freeText, "year:abc")
    }

    func testParseBareOperators() {
        XCTAssertTrue(QueryParser.parse("/").isEmpty)
        XCTAssertTrue(QueryParser.parse("@").isEmpty)
        XCTAssertTrue(QueryParser.parse("!").isEmpty)
        XCTAssertTrue(QueryParser.parse("%").isEmpty)
    }

    func testEscapeFTS5() {
        let escaped = QueryParser.escapeFTS5("hello*world\"test")
        XCTAssertEqual(escaped, "helloworldtest")
    }

    func testEscapeFTS5HyphenReplacedWithSpace() {
        // `-` is the FTS5 NOT operator; replace with space to align with unicode61 tokenizer
        let escaped = QueryParser.escapeFTS5("-something")
        XCTAssertEqual(escaped, " something")
        let escaped2 = QueryParser.escapeFTS5("hip-hop")
        XCTAssertEqual(escaped2, "hip hop")
    }

    func testEscapeFTS5KeywordsNeutralizedByQuoting() {
        // OR/AND/NOT are FTS5 keywords — escapeFTS5 strips metacharacters,
        // and SearchEngine wraps tokens in quotes to neutralize keywords
        let escaped = QueryParser.escapeFTS5("rock OR metal")
        XCTAssertEqual(escaped, "rock OR metal") // escapeFTS5 doesn't strip keywords
        // The quoting happens at the SearchEngine call site: "rock"* "OR"* "metal"*
    }

    func testDefaultSearchDoesNotProduceTrackSearch() {
        let q = QueryParser.parse("radiohead")
        XCTAssertFalse(q.hasTrackSearch)
        XCTAssertEqual(q.freeText, "radiohead")
    }

    func testIsFreeTextOnly() {
        XCTAssertTrue(QueryParser.parse("radiohead").isFreeTextOnly)
        XCTAssertFalse(QueryParser.parse("@radiohead").isFreeTextOnly)
        XCTAssertFalse(QueryParser.parse("!paranoid").isFreeTextOnly)
        XCTAssertFalse(QueryParser.parse("/rock AND radiohead").isFreeTextOnly)
        XCTAssertFalse(QueryParser.parse("").isFreeTextOnly)
    }
}

// MARK: - SearchEngine Tests

final class SearchEngineTests: XCTestCase {

    private var db: CacheDatabase!
    private var engine: SearchEngine!

    override func setUpWithError() throws {
        db = try CacheDatabase.temporary()
        engine = SearchEngine(db: db)
        try seedTestData()
    }

    private func seedTestData() throws {
        let radioheadId = try db.upsertArtist(
            name: "Radiohead", sortName: nil, sourceId: "artist-1",
            artUrl: nil, summary: nil, updatedAt: nil
        )
        let slayerId = try db.upsertArtist(
            name: "Slayer", sortName: nil, sourceId: "artist-2",
            artUrl: nil, summary: nil, updatedAt: nil
        )

        let okComputerId = try db.upsertAlbum(
            title: "OK Computer", artistId: radioheadId, year: 1997,
            sourceId: "album-1", artUrl: nil, updatedAt: nil
        )
        let reignId = try db.upsertAlbum(
            title: "Reign in Blood", artistId: slayerId, year: 1986,
            sourceId: "album-2", artUrl: nil, updatedAt: nil
        )
        let kidAId = try db.upsertAlbum(
            title: "Kid A", artistId: radioheadId, year: 2000,
            sourceId: "album-3", artUrl: nil, updatedAt: nil
        )

        try db.upsertTrack(
            title: "Paranoid Android", albumId: okComputerId, artistId: radioheadId,
            trackNumber: 1, discNumber: 1, durationMs: 384000,
            sourceId: "track-1", codec: "flac", partKey: nil, streamId: nil, updatedAt: nil
        )
        try db.upsertTrack(
            title: "Karma Police", albumId: okComputerId, artistId: radioheadId,
            trackNumber: 2, discNumber: 1, durationMs: 264000,
            sourceId: "track-2", codec: "flac", partKey: nil, streamId: nil, updatedAt: nil
        )
        try db.upsertTrack(
            title: "Angel of Death", albumId: reignId, artistId: slayerId,
            trackNumber: 1, discNumber: 1, durationMs: 294000,
            sourceId: "track-3", codec: "flac", partKey: nil, streamId: nil, updatedAt: nil
        )
        try db.upsertTrack(
            title: "Raining Blood", albumId: reignId, artistId: slayerId,
            trackNumber: 2, discNumber: 1, durationMs: 252000,
            sourceId: "track-4", codec: "flac", partKey: nil, streamId: nil, updatedAt: nil
        )
        try db.upsertTrack(
            title: "Everything In Its Right Place", albumId: kidAId, artistId: radioheadId,
            trackNumber: 1, discNumber: 1, durationMs: 250000,
            sourceId: "track-5", codec: "flac", partKey: nil, streamId: nil, updatedAt: nil
        )

        let rockId = try db.upsertGenre(name: "Rock")
        let metalId = try db.upsertGenre(name: "Metal")
        let electronicId = try db.upsertGenre(name: "Electronic")

        try db.setAlbumGenres(albumId: okComputerId, genreIds: [rockId])
        try db.setAlbumGenres(albumId: reignId, genreIds: [metalId])
        try db.setAlbumGenres(albumId: kidAId, genreIds: [rockId, electronicId])
    }

    // MARK: - Album Search (default)

    func testFreeTextSearchReturnsAlbumsAndTracks() throws {
        let q = QueryParser.parse("radiohead")
        let results = try engine.search(q)
        let albums = results.filter { $0.kind == .album }
        let tracks = results.filter { $0.kind == .track }
        XCTAssertFalse(albums.isEmpty, "Should have album results")
        XCTAssertTrue(albums.allSatisfy { $0.artistName == "Radiohead" })
        XCTAssertFalse(tracks.isEmpty, "Free text should also return tracks")
    }

    func testArtistFilterReturnsAlbums() throws {
        let q = QueryParser.parse("@slayer")
        let results = try engine.search(q)
        XCTAssertFalse(results.isEmpty)
        XCTAssertTrue(results.allSatisfy { $0.kind == .album })
        XCTAssertTrue(results.allSatisfy { $0.artistName == "Slayer" })
    }

    func testAlbumTitleFilter() throws {
        let q = QueryParser.parse("%ok computer")
        let results = try engine.search(q)
        XCTAssertFalse(results.isEmpty)
        XCTAssertTrue(results.allSatisfy { $0.kind == .album })
        XCTAssertEqual(results.first?.albumTitle, "OK Computer")
    }

    func testGenreFilterReturnsAlbums() throws {
        let q = QueryParser.parse("/rock")
        let results = try engine.search(q)
        XCTAssertFalse(results.isEmpty)
        XCTAssertTrue(results.allSatisfy { $0.kind == .album })
        let titles = Set(results.map(\.albumTitle))
        XCTAssertTrue(titles.contains("OK Computer"))
        XCTAssertTrue(titles.contains("Kid A"))
        XCTAssertFalse(titles.contains("Reign in Blood"))
    }

    func testGenreFilterExpandsHierarchy() throws {
        // Create a genre hierarchy where "Rock" has "Electronic" as a child
        let genreJSON = """
        {"genres":[{"name":"Rock","children":[{"name":"Electronic","children":[]}]},{"name":"Metal","children":[]}]}
        """
        let tempURL = FileManager.default.temporaryDirectory.appendingPathComponent("test_genres.json")
        try genreJSON.data(using: .utf8)!.write(to: tempURL)
        defer { try? FileManager.default.removeItem(at: tempURL) }

        let mapper = try GenreMapper(jsonURL: tempURL)
        let hierarchyEngine = SearchEngine(db: db, genreMapper: mapper)

        // /rock should find albums tagged "Rock" AND "Electronic" (child of Rock in our test hierarchy)
        let q = QueryParser.parse("/rock")
        let results = try hierarchyEngine.search(q)
        let titles = Set(results.map(\.albumTitle))
        XCTAssertTrue(titles.contains("OK Computer"), "Should include Rock-tagged album")
        XCTAssertTrue(titles.contains("Kid A"), "Should include Electronic-tagged album (child of Rock)")
        XCTAssertFalse(titles.contains("Reign in Blood"), "Should not include Metal-tagged album")
    }

    func testYearRangeFilter() throws {
        let q = QueryParser.parse("year:>1999")
        let results = try engine.search(q)
        XCTAssertFalse(results.isEmpty)
        XCTAssertTrue(results.allSatisfy { $0.albumTitle == "Kid A" })
    }

    func testCombinedFilters() throws {
        let q = QueryParser.parse("/rock AND year:>1999")
        let results = try engine.search(q)
        XCTAssertFalse(results.isEmpty)
        XCTAssertTrue(results.allSatisfy { $0.albumTitle == "Kid A" })
    }

    // MARK: - Track Search (! operator)

    func testTrackSearchReturnsTracksOnly() throws {
        let q = QueryParser.parse("!paranoid")
        let results = try engine.search(q)
        let tracks = results.filter { $0.kind == .track }
        XCTAssertFalse(tracks.isEmpty)
        XCTAssertEqual(tracks.first?.trackTitle, "Paranoid Android")
    }

    func testTrackSearchFuzzyFallback() throws {
        let q = QueryParser.parse("!paranoyd")
        let results = try engine.search(q)
        let tracks = results.filter { $0.kind == .track }
        XCTAssertFalse(tracks.isEmpty, "Fuzzy should find 'Paranoid Android' for typo 'paranoyd'")
    }

    func testFreeTextAlbumsAppearBeforeTracks() throws {
        let q = QueryParser.parse("radiohead")
        let results = try engine.search(q)
        let firstTrackIndex = results.firstIndex { $0.kind == .track }
        let lastAlbumIndex = results.lastIndex { $0.kind == .album }
        if let ti = firstTrackIndex, let ai = lastAlbumIndex {
            XCTAssertGreaterThan(ti, ai, "All albums should appear before any track")
        }
    }

    func testFreeTextGibberishReturnsEmpty() throws {
        let q = QueryParser.parse("zzzznonexistent")
        let results = try engine.search(q)
        XCTAssertTrue(results.isEmpty, "No results for gibberish query")
    }

    func testFreeTextAlbumsCappedAtFive() throws {
        let q = QueryParser.parse("radiohead")
        let results = try engine.search(q)
        let albums = results.filter { $0.kind == .album }
        XCTAssertLessThanOrEqual(albums.count, 5)
    }

    func testEmptyQuery() throws {
        let q = QueryParser.parse("")
        let results = try engine.search(q)
        XCTAssertTrue(results.isEmpty)
    }
}
