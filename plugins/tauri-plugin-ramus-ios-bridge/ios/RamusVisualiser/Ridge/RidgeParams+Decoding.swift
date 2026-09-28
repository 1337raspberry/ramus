import Foundation

/// `RidgeParams` from the frontend's `VISUALIZER_PARAMS` object
/// (`ui/src/lib/visualizerParams.ts`), which shares its key names. Keys the
/// ridge doesn't use are ignored. A key that is missing, `null` or not a
/// number keeps the standard value, and whole-number fields take the
/// nearest whole number, so a live-tuned value can never fail the decode.
/// A whole-number field outside its sane range (including a finite value
/// too large to fit an `Int`, which would otherwise trap converting it)
/// also keeps the standard value rather than taking the out-of-range one.
extension RidgeParams: Decodable {
    private enum CodingKeys: String, CodingKey {
        case ridgeRows, ridgeFullScreenRows, ridgeHeight, ridgePeak, ridgeDepthScale
        case ridgeBottom, ridgeSpan, ridgeLineWidth, ridgeOversample, ridgeAlpha
        case ridgeBackAlpha, ridgeFadeCurve, ridgeRowMs, ridgeSpread, ridgeSmooth
        case ridgeGrain, ridgeEdgeTaper, ridgeAttack, ridgeDecay, ridgeGamma
        case ridgeFloorCut, ridgeGain, ridgeSyncLeadMs, ridgeAxisCurve
    }

    /// Rows are the display's history stack and grow one allocation per row;
    /// 1024 is far past anything the UI offers but rules out a fat-fingered
    /// or garbled value trying to allocate a huge stack.
    private static let rowRange = 1...1024
    /// `RidgePainter.step` already clamps the points-per-band it resamples
    /// to at most 8; this only needs to keep the decoded value representable
    /// and plausible ahead of that clamp.
    private static let oversampleRange = 1...32

    init(from decoder: Decoder) throws {
        self.init()
        let c = try decoder.container(keyedBy: CodingKeys.self)
        func number(_ key: CodingKeys) -> Double? {
            guard let value = (try? c.decodeIfPresent(Double.self, forKey: key)) ?? nil,
                  value.isFinite
            else { return nil }
            return value
        }
        func read(_ key: CodingKeys, into field: inout Double) {
            if let v = number(key) { field = v }
        }
        // `Int(exactly:)` returns nil instead of trapping when the rounded
        // value doesn't fit an `Int` (e.g. a stray 1e300); `range` then
        // rejects anything outside the field's sane bounds the same way.
        func read(_ key: CodingKeys, into field: inout Int, range: ClosedRange<Int>) {
            guard let v = number(key), let i = Int(exactly: v.rounded()), range.contains(i) else { return }
            field = i
        }
        read(.ridgeRows, into: &ridgeRows, range: Self.rowRange)
        read(.ridgeFullScreenRows, into: &ridgeFullScreenRows, range: Self.rowRange)
        read(.ridgeHeight, into: &ridgeHeight)
        read(.ridgePeak, into: &ridgePeak)
        read(.ridgeDepthScale, into: &ridgeDepthScale)
        read(.ridgeBottom, into: &ridgeBottom)
        read(.ridgeSpan, into: &ridgeSpan)
        read(.ridgeLineWidth, into: &ridgeLineWidth)
        read(.ridgeOversample, into: &ridgeOversample, range: Self.oversampleRange)
        read(.ridgeAlpha, into: &ridgeAlpha)
        read(.ridgeBackAlpha, into: &ridgeBackAlpha)
        read(.ridgeFadeCurve, into: &ridgeFadeCurve)
        read(.ridgeRowMs, into: &ridgeRowMs)
        read(.ridgeSpread, into: &ridgeSpread)
        read(.ridgeSmooth, into: &ridgeSmooth)
        read(.ridgeGrain, into: &ridgeGrain)
        read(.ridgeEdgeTaper, into: &ridgeEdgeTaper)
        read(.ridgeAttack, into: &ridgeAttack)
        read(.ridgeDecay, into: &ridgeDecay)
        read(.ridgeGamma, into: &ridgeGamma)
        read(.ridgeFloorCut, into: &ridgeFloorCut)
        read(.ridgeGain, into: &ridgeGain)
        read(.ridgeSyncLeadMs, into: &ridgeSyncLeadMs)
        read(.ridgeAxisCurve, into: &ridgeAxisCurve)
    }
}
