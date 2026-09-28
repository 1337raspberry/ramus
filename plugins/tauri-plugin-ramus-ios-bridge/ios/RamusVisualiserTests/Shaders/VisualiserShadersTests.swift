import Metal
import XCTest
@testable import RamusVisualiser

/// The shader sources compile at runtime and expose the functions the
/// renderers build their pipelines from.
final class VisualiserShadersTests: XCTestCase {
    private var device: MTLDevice!

    override func setUpWithError() throws {
        device = try XCTUnwrap(MTLCreateSystemDefaultDevice(), "Metal is unavailable")
    }

    func testTheRidgeLibraryHasItsFunctions() throws {
        let library = try VisualiserShaders.library(.ridge, on: device)
        for name in ["ridgeEraseVertex", "ridgeEraseFragment", "ridgeStrokeVertex", "ridgeStrokeFragment"] {
            XCTAssertNotNil(library.makeFunction(name: name), name)
        }
    }

    func testTheBackdropLibraryHasItsFunctions() throws {
        let library = try VisualiserShaders.library(.backdrop, on: device)
        for name in ["backdropVertex", "backdropFieldFragment", "backdropDitherFragment", "backdropFragment"] {
            XCTAssertNotNil(library.makeFunction(name: name), name)
        }
    }

    func testALibraryIsCompiledOncePerDevice() throws {
        let first = try VisualiserShaders.library(.ridge, on: device)
        let second = try VisualiserShaders.library(.ridge, on: device)
        XCTAssertTrue((first as AnyObject) === (second as AnyObject))
    }

    func testPrepareSucceeds() throws {
        XCTAssertNoThrow(try VisualiserShaders.prepare())
    }
}
