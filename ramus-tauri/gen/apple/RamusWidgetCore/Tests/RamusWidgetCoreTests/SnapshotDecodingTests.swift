import XCTest
@testable import RamusWidgetCore

final class SnapshotDecodingTests: XCTestCase {
    func testDecodesTheRustLayout() throws {
        let s = try WidgetSnapshot.decode(snapshotJSON())
        XCTAssertEqual(s.state, .playing)
        XCTAssertEqual(s.track?.ratingKey, "9")
        XCTAssertEqual(s.track?.durationSec, 200)
        XCTAssertEqual(s.colors?.accent, "3ddb7a")
        XCTAssertEqual(s.waveform, [0.1, 0.5, 1.0])
        XCTAssertEqual(s.revision, 4)
    }

    func testMissingFileIsNil() {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        XCTAssertNil(WidgetSnapshot.load(from: dir))
        XCTAssertEqual(TimelinePlanner.plan(nil, now: at(5)), [PlannedEntry(date: at(5), face: .idle(resume: false))])
    }

    func testTornJsonIsNil() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let full = snapshotJSON()
        try full.prefix(full.count / 2).write(to: dir.appendingPathComponent(SharedContainer.snapshotFile))
        XCTAssertNil(WidgetSnapshot.load(from: dir))
        try full.write(to: dir.appendingPathComponent(SharedContainer.snapshotFile))
        XCTAssertNotNil(WidgetSnapshot.load(from: dir))
    }

    func testSchemaTwoIsNil() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try snapshotJSON(schema: 2).write(to: dir.appendingPathComponent(SharedContainer.snapshotFile))
        XCTAssertNil(WidgetSnapshot.load(from: dir))
    }
}
