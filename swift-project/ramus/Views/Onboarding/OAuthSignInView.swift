import SwiftUI

/// Step 1: Sign in with Plex via OAuth browser flow.
struct OAuthSignInView: View {

    @Bindable var vm: OnboardingViewModel

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "music.note.house")
                .font(.system(size: 48))
                .foregroundStyle(.tint)

            Text("Welcome to ramus")
                .font(.largeTitle)
                .fontWeight(.bold)

            Text("Sign in with your Plex account to get started.")
                .foregroundStyle(.secondary)

            if vm.isAuthenticating {
                VStack(spacing: 12) {
                    ProgressView()
                        .controlSize(.regular)
                    Text("Waiting for authorization...")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                    if let code = vm.pinCode {
                        Text("Code: \(code)")
                            .font(.system(.body, design: .monospaced))
                            .foregroundStyle(.tertiary)
                    }
                }
                .padding(.top, 8)
            } else {
                Button("Sign in with Plex") {
                    Task { await vm.startOAuth() }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
            }

            if let error = vm.oauthError {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .multilineTextAlignment(.center)
            }

        }
        .onboardingCard()
    }
}
