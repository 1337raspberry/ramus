import XCTest
@testable import RamusVisualiser

/// `RidgeParams` decodes from the frontend's `VISUALIZER_PARAMS` object,
/// whose keys it shares, and falls back to its own value for any key that
/// is missing or unusable.
final class RidgeParamsDecodingTests: XCTestCase {
    private func decode(_ json: String) throws -> RidgeParams {
        try JSONDecoder().decode(RidgeParams.self, from: Data(json.utf8))
    }

    func testEveryKeyIsRead() throws {
        let json = """
        {"ridgeRows":40,"ridgeFullScreenRows":60,"ridgeHeight":0.5,"ridgePeak":0.4,
         "ridgeDepthScale":0.3,"ridgeBottom":0.02,"ridgeSpan":0.9,"ridgeLineWidth":2,
         "ridgeOversample":4,"ridgeAlpha":0.6,"ridgeBackAlpha":0.1,"ridgeFadeCurve":1.5,
         "ridgeRowMs":40,"ridgeSpread":2,"ridgeSmooth":0.2,"ridgeGrain":0.05,
         "ridgeEdgeTaper":0.1,"ridgeAttack":0.5,"ridgeDecay":0.4,"ridgeGamma":3,
         "ridgeFloorCut":0.5,"ridgeGain":1.2,"ridgeSyncLeadMs":30,"ridgeAxisCurve":0.8,
         "barAlpha":0.3,"syncLeadMs":60}
        """
        let p = try decode(json)
        XCTAssertEqual(p.ridgeRows, 40)
        XCTAssertEqual(p.ridgeFullScreenRows, 60)
        XCTAssertEqual(p.ridgeHeight, 0.5)
        XCTAssertEqual(p.ridgePeak, 0.4)
        XCTAssertEqual(p.ridgeDepthScale, 0.3)
        XCTAssertEqual(p.ridgeBottom, 0.02)
        XCTAssertEqual(p.ridgeSpan, 0.9)
        XCTAssertEqual(p.ridgeLineWidth, 2)
        XCTAssertEqual(p.ridgeOversample, 4)
        XCTAssertEqual(p.ridgeAlpha, 0.6)
        XCTAssertEqual(p.ridgeBackAlpha, 0.1)
        XCTAssertEqual(p.ridgeFadeCurve, 1.5)
        XCTAssertEqual(p.ridgeRowMs, 40)
        XCTAssertEqual(p.ridgeSpread, 2)
        XCTAssertEqual(p.ridgeSmooth, 0.2)
        XCTAssertEqual(p.ridgeGrain, 0.05)
        XCTAssertEqual(p.ridgeEdgeTaper, 0.1)
        XCTAssertEqual(p.ridgeAttack, 0.5)
        XCTAssertEqual(p.ridgeDecay, 0.4)
        XCTAssertEqual(p.ridgeGamma, 3)
        XCTAssertEqual(p.ridgeFloorCut, 0.5)
        XCTAssertEqual(p.ridgeGain, 1.2)
        XCTAssertEqual(p.ridgeSyncLeadMs, 30)
        XCTAssertEqual(p.ridgeAxisCurve, 0.8)
    }

    func testAMissingKeyKeepsItsDefault() throws {
        let p = try decode(#"{"ridgeRows":40}"#)
        var expected = RidgeParams.standard
        expected.ridgeRows = 40
        XCTAssertEqual(p, expected)
    }

    /// A live-tuned slider can leave a whole-number field fractional, and a
    /// stray value of the wrong type must not fail the whole object.
    func testDecodingIsLenient() throws {
        let p = try decode(#"{"ridgeRows":51.4,"ridgeOversample":4.6,"ridgePeak":"tall","ridgeGain":null}"#)
        XCTAssertEqual(p.ridgeRows, 51)
        XCTAssertEqual(p.ridgeOversample, 5)
        XCTAssertEqual(p.ridgePeak, RidgeParams.standard.ridgePeak)
        XCTAssertEqual(p.ridgeGain, RidgeParams.standard.ridgeGain)
    }

    /// A finite value too large to fit an `Int` would trap converting it
    /// with the plain initialiser; the decoder must fall back instead.
    func testAHugeWholeNumberFallsBackToTheDefault() throws {
        let p = try decode(#"{"ridgeRows":1e300,"ridgeFullScreenRows":1e300,"ridgeOversample":1e300}"#)
        XCTAssertEqual(p.ridgeRows, RidgeParams.standard.ridgeRows)
        XCTAssertEqual(p.ridgeFullScreenRows, RidgeParams.standard.ridgeFullScreenRows)
        XCTAssertEqual(p.ridgeOversample, RidgeParams.standard.ridgeOversample)
    }

    /// Zero or negative rows/oversample are in range for `Int` but make no
    /// sense for the stack or the resample, so they fall back too.
    func testANonPositiveWholeNumberFallsBackToTheDefault() throws {
        let p = try decode(#"{"ridgeRows":-5,"ridgeFullScreenRows":-5,"ridgeOversample":-5}"#)
        XCTAssertEqual(p.ridgeRows, RidgeParams.standard.ridgeRows)
        XCTAssertEqual(p.ridgeFullScreenRows, RidgeParams.standard.ridgeFullScreenRows)
        XCTAssertEqual(p.ridgeOversample, RidgeParams.standard.ridgeOversample)
    }

    /// In range for `Int`, and positive, but past the field's own sane
    /// bound — still a fall back, not a giant stack or resample.
    func testAnOutOfRangeWholeNumberFallsBackToTheDefault() throws {
        let p = try decode(#"{"ridgeRows":5000,"ridgeFullScreenRows":5000,"ridgeOversample":5000}"#)
        XCTAssertEqual(p.ridgeRows, RidgeParams.standard.ridgeRows)
        XCTAssertEqual(p.ridgeFullScreenRows, RidgeParams.standard.ridgeFullScreenRows)
        XCTAssertEqual(p.ridgeOversample, RidgeParams.standard.ridgeOversample)
    }

    func testAnEmptyObjectIsTheStandardLook() throws {
        XCTAssertEqual(try decode("{}"), .standard)
    }
}
