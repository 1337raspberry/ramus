// Ported from ramusTV `RamusTVTests/Backdrop/BackdropAnimatorTests.swift`.

import XCTest
@testable import RamusVisualiser

final class BackdropAnimatorTests: XCTestCase {
    private func flat(_ v: Double) -> BackdropFieldColors {
        BackdropFieldColors(topLeft: SIMD3(repeating: v), topRight: SIMD3(repeating: v),
                            bottomLeft: SIMD3(repeating: v), bottomRight: SIMD3(repeating: v))
    }

    func testTheFirstColoursPaintAtOnce() {
        var a = BackdropAnimator()
        XCTAssertNil(a.shown)
        XCTAssertTrue(a.setColors(flat(10), now: 0, animated: true))
        XCTAssertEqual(a.shown, flat(10))
        XCTAssertFalse(a.isAnimating)
    }

    func testNewColoursCrossfadeOnTheEasedCurve() {
        var a = BackdropAnimator()
        _ = a.setColors(flat(10), now: 0, animated: true)
        XCTAssertTrue(a.setColors(flat(110), now: 10, animated: true))
        XCTAssertTrue(a.isAnimating)
        XCTAssertTrue(a.step(now: 10.2))
        XCTAssertEqual(a.shown, flat(10).mixed(to: flat(110), BackdropField.easeInOut(0.25)))
        XCTAssertFalse(a.step(now: 10.8))
        XCTAssertEqual(a.shown, flat(110))
        XCTAssertFalse(a.isAnimating)
    }

    func testTheSameColoursAgainDoNothing() {
        var a = BackdropAnimator()
        _ = a.setColors(flat(10), now: 0, animated: true)
        XCTAssertFalse(a.setColors(flat(10), now: 1, animated: true))
        XCTAssertFalse(a.isAnimating)
    }

    func testAChangeMidFadeStartsFromTheColoursOnScreen() {
        var a = BackdropAnimator()
        _ = a.setColors(flat(10), now: 0, animated: true)
        _ = a.setColors(flat(110), now: 1, animated: true)
        _ = a.step(now: 1.4)
        let middle = a.shown
        _ = a.setColors(flat(0), now: 1.4, animated: true)
        _ = a.step(now: 1.4)
        XCTAssertEqual(a.shown, middle)
        _ = a.step(now: 2.2)
        XCTAssertEqual(a.shown, flat(0))
    }

    func testUnanimatedColoursJump() {
        var a = BackdropAnimator()
        _ = a.setColors(flat(10), now: 0, animated: true)
        XCTAssertTrue(a.setColors(flat(110), now: 1, animated: false))
        XCTAssertEqual(a.shown, flat(110))
        XCTAssertFalse(a.isAnimating)
    }

    func testStrengthFadesOverTheStrengthDuration() {
        var a = BackdropAnimator()
        XCTAssertEqual(a.strength, 1)
        XCTAssertTrue(a.setStrength(0, now: 5, animated: true))
        XCTAssertTrue(a.isAnimating)
        XCTAssertTrue(a.step(now: 5.1))
        XCTAssertEqual(a.strength, 1 - BackdropField.easeInOut(0.1 / BackdropAnimator.strengthSeconds), accuracy: 1e-12)
        XCTAssertFalse(a.step(now: 5 + BackdropAnimator.strengthSeconds))
        XCTAssertEqual(a.strength, 0)
        XCTAssertFalse(a.setStrength(0, now: 6, animated: true))
    }

    func testUnanimatedStrengthJumps() {
        var a = BackdropAnimator()
        XCTAssertTrue(a.setStrength(0, now: 0, animated: false))
        XCTAssertEqual(a.strength, 0)
        XCTAssertFalse(a.isAnimating)
    }
}
