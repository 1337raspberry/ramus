import SwiftUI

/// Shared card styling for all onboarding wizard steps.
struct OnboardingCardModifier: ViewModifier {
    func body(content: Content) -> some View {
        content
            .frame(width: 450)
            .padding(32)
            .background(.black.opacity(0.3), in: RoundedRectangle(cornerRadius: 16))
            .compatGlassClear(in: RoundedRectangle(cornerRadius: 16))
    }
}

extension View {
    func onboardingCard() -> some View {
        modifier(OnboardingCardModifier())
    }
}
