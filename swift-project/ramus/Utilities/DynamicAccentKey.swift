import SwiftUI

private struct DynamicAccentKey: EnvironmentKey {
    static let defaultValue: Color = .accentColor
}

extension EnvironmentValues {
    var dynamicAccent: Color {
        get { self[DynamicAccentKey.self] }
        set { self[DynamicAccentKey.self] = newValue }
    }
}
