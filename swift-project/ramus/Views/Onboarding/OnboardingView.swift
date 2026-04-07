import SwiftUI
import PlexAPI
import Cache
import Models

/// Multi-step onboarding wizard. Hands off configured objects on completion.
struct OnboardingView: View {

    @State private var vm = OnboardingViewModel()
    var onComplete: (PlexClient, CacheDatabase?, LibrarySection, PlexServer?) -> Void

    var body: some View {
        @Bindable var vmBinding = vm
        VStack {
            Spacer()

            Group {
                switch vm.step {
                case .signIn:
                    OAuthSignInView(vm: vm)
                case .discoverServers:
                    ServerPickerView(vm: vm)
                case .selectLibrary:
                    LibraryPickerView(vm: vm)
                case .initialSync:
                    InitialSyncView(vm: vm, onComplete: {
                        guard let library = vm.selectedLibrary else { return }
                        _ = vm.finalize()
                        onComplete(vm.plexClient, vm.cache, library, vm.selectedServer)
                    })
                }
            }
            .animation(.easeInOut(duration: 0.3), value: vm.step)

            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .alert("Unencrypted Connection", isPresented: $vmBinding.showHTTPWarning) {
            Button("Continue for now", role: .cancel) { }
        } message: {
            Text("Your Plex server is only reachable over HTTP. Your auth token will be sent unencrypted on this network.\n\nIf this is your home network and you're connected to your own Plex server, this is probably fine — but you should try to get HTTPS set up.")
        }
    }
}
