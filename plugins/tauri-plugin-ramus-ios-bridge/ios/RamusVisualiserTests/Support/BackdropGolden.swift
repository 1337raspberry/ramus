// Ported from ramusTV `RamusTVTests/Support/BackdropGolden.swift`.

import XCTest

/// `Fixtures/backdrop-golden.json`: ramus's own corner
/// extraction on synthetic images, and its field constants and easing
/// samples, written by `tools/backdrop-golden/golden.mjs`.
struct BackdropGolden: Decodable {
    struct Corners: Decodable {
        let topLeft: String
        let topRight: String
        let bottomLeft: String
        let bottomRight: String
    }

    struct Image: Decodable {
        let name: String
        let size: Int
        let rgba: [UInt8]
        let corners: Corners

        private enum CodingKeys: String, CodingKey { case name, size, rgba, corners }

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            name = try c.decode(String.self, forKey: .name)
            size = try c.decode(Int.self, forKey: .size)
            corners = try c.decode(Corners.self, forKey: .corners)
            let base64 = try c.decode(String.self, forKey: .rgba)
            guard let data = Data(base64Encoded: base64) else {
                throw DecodingError.dataCorruptedError(forKey: .rgba, in: c, debugDescription: "not base64")
            }
            rgba = [UInt8](data)
        }
    }

    let source: String
    let fieldBase: [Double]
    let falloffStops: [[Double]]
    let transitionMs: Double
    let easeInOut: [[Double]]
    let images: [Image]

    static func load() throws -> BackdropGolden {
        let url = try XCTUnwrap(
            Bundle.module.url(forResource: "backdrop-golden", withExtension: "json", subdirectory: "Fixtures"),
            "missing fixture backdrop-golden.json")
        return try JSONDecoder().decode(BackdropGolden.self, from: Data(contentsOf: url))
    }
}
