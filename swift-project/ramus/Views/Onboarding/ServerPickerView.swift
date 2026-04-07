import SwiftUI
import Models

/// Step 2: Discover and select a Plex server.
struct ServerPickerView: View {

    @Bindable var vm: OnboardingViewModel

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "server.rack")
                .font(.system(size: 36))
                .foregroundStyle(.tint)

            Text("Select Server")
                .font(.title2)
                .fontWeight(.semibold)

            if vm.isDiscovering {
                VStack(spacing: 8) {
                    ProgressView()
                        .controlSize(.regular)
                    Text("Discovering servers...")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
            } else if vm.servers.isEmpty && !vm.showManualURL {
                Text("No servers found.")
                    .foregroundStyle(.secondary)

                Button("Retry") {
                    Task { await vm.discoverServers() }
                }
                .buttonStyle(.bordered)

                Button("Enter URL manually") {
                    vm.showManualURL = true
                }
                .buttonStyle(.borderless)
                .font(.caption)
            } else {
                VStack(spacing: 8) {
                    ForEach(vm.servers) { server in
                        serverRow(server)
                    }
                }

                if vm.isTestingConnections {
                    HStack(spacing: 8) {
                        ProgressView()
                            .controlSize(.small)
                        Text("Testing connections...")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                if let type = vm.connectionType, !vm.isTestingConnections {
                    Label("Connected via \(type)", systemImage: "checkmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.green)
                }
            }

            if vm.showManualURL {
                manualURLEntry
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

    private func serverRow(_ server: PlexServer) -> some View {
        Button {
            Task { await vm.selectServer(server) }
        } label: {
            HStack {
                Image(systemName: server.owned ? "externaldrive.fill" : "externaldrive")
                    .foregroundStyle(.tint)
                VStack(alignment: .leading, spacing: 2) {
                    Text(server.name)
                        .fontWeight(.medium)
                    HStack(spacing: 4) {
                        if server.owned {
                            Text("Owned")
                        } else {
                            Text("Shared")
                        }
                        Text("·")
                        Text("\(server.connections.count) connections")
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
                Spacer()
                if vm.selectedServer?.id == server.id {
                    Image(systemName: "checkmark")
                        .foregroundStyle(.tint)
                }
            }
            .padding(10)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .fill(vm.selectedServer?.id == server.id
                          ? Color.accentColor.opacity(0.15)
                          : Color.white.opacity(0.05))
            )
        }
        .buttonStyle(.plain)
        .disabled(vm.isTestingConnections)
    }

    private var manualURLEntry: some View {
        VStack(spacing: 8) {
            Divider()
                .frame(width: 200)

            Text("Enter server URL")
                .font(.caption)
                .foregroundStyle(.secondary)

            HStack {
                TextField("http://192.168.1.x:32400", text: $vm.manualURLInput)
                    .textFieldStyle(.roundedBorder)
                    .frame(maxWidth: 280)
                    .onSubmit { Task { await vm.connectManualURL() } }

                Button("Connect") {
                    Task { await vm.connectManualURL() }
                }
                .disabled(vm.manualURLInput.isEmpty)
            }

            if let error = vm.manualURLError {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
    }
}
