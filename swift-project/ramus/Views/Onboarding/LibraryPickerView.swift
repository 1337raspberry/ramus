import SwiftUI
import PlexAPI
import Models

/// Step 3: Select a music library from the connected server.
struct LibraryPickerView: View {

    @Bindable var vm: OnboardingViewModel

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "music.note.list")
                .font(.system(size: 36))
                .foregroundStyle(.tint)

            Text("Select Library")
                .font(.title2)
                .fontWeight(.semibold)

            if let server = vm.selectedServer {
                Text("on \(server.name)")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
            }

            VStack(spacing: 8) {
                ForEach(vm.musicLibraries, id: \.key) { lib in
                    Button {
                        vm.selectLibrary(lib)
                    } label: {
                        HStack {
                            Image(systemName: "music.quarternote.3")
                                .foregroundStyle(.tint)
                            Text(lib.title)
                                .fontWeight(.medium)
                            Spacer()
                            if vm.selectedLibrary?.key == lib.key {
                                Image(systemName: "checkmark")
                                    .foregroundStyle(.tint)
                            }
                        }
                        .padding(10)
                        .background(
                            RoundedRectangle(cornerRadius: 8)
                                .fill(vm.selectedLibrary?.key == lib.key
                                      ? Color.accentColor.opacity(0.15)
                                      : Color.white.opacity(0.05))
                        )
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .onboardingCard()
    }
}
