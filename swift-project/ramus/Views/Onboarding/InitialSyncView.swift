import SwiftUI
import Cache

/// Step 4: Initial library sync with explanation and progress.
struct InitialSyncView: View {

    @Bindable var vm: OnboardingViewModel
    var onComplete: () -> Void

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "arrow.triangle.2.circlepath")
                .font(.system(size: 36))
                .foregroundStyle(.tint)

            Text("Sync Your Library")
                .font(.title2)
                .fontWeight(.semibold)

            VStack(spacing: 8) {
                Text("ramus syncs some of your library metadata locally for instant genre browsing and search.")
                Text("Your music files stay on your server.")
                Text("This sync will quietly run periodically in the background. You can adjust the interval, force a sync, or do a full resync from the settings menu.")
            }
            .font(.subheadline)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)

            if vm.isSyncing {
                VStack(spacing: 8) {
                    if let progress = vm.syncProgress {
                        ProgressView(value: progress.fraction) {
                            Text(progress.detail)
                                .font(.caption)
                        }
                        .frame(maxWidth: 300)

                        Text(progress.phase.displayName)
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                    } else {
                        ProgressView()
                            .controlSize(.regular)
                    }
                }
                .padding(.top, 8)
            } else if vm.syncProgress?.phase == .done {
                Label("Sync complete!", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)

                Button("Get Started") {
                    onComplete()
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
            } else {
                Text("This usually takes 1-3 minutes depending on library size.")
                    .font(.caption)
                    .foregroundStyle(.tertiary)

                Button("Start Sync") {
                    Task { await vm.startInitialSync() }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)

                Button("Skip for now") {
                    vm.skipSync()
                    onComplete()
                }
                .buttonStyle(.borderless)
                .font(.caption)
                .foregroundStyle(.secondary)
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
