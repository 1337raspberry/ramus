import Foundation
import QuartzCore

/// The display rates the ridge can run at. 60 is the default; 120 needs a
/// ProMotion display and `CADisableMinimumFrameDurationOnPhone` in the
/// app's Info.plist, and otherwise delivers whatever the display allows.
enum RidgeFrameRate: Int, CaseIterable {
    case sixty = 60
    case oneTwenty = 120

    /// `UserDefaults` key the chosen rate is kept under.
    static let defaultsKey = "RamusVisualiserFrameRate"

    /// The display-link request for this rate.
    var range: CAFrameRateRange {
        switch self {
        case .sixty: return CAFrameRateRange(minimum: 60, maximum: 60, preferred: 60)
        case .oneTwenty: return CAFrameRateRange(minimum: 80, maximum: 120, preferred: 120)
        }
    }

    /// The other rate.
    var toggled: RidgeFrameRate {
        self == .sixty ? .oneTwenty : .sixty
    }

    /// The stored rate; 60 when nothing, or an unknown value, is stored.
    static func load(_ defaults: UserDefaults = .standard) -> RidgeFrameRate {
        RidgeFrameRate(rawValue: defaults.integer(forKey: defaultsKey)) ?? .sixty
    }

    func save(_ defaults: UserDefaults = .standard) {
        defaults.set(rawValue, forKey: Self.defaultsKey)
    }
}
