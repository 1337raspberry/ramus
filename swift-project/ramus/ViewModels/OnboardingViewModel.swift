import SwiftUI
import PlexAPI
import Cache
import Models

/// Manages the multi-step onboarding flow: OAuth → server discovery → library selection → initial sync.
@MainActor @Observable
final class OnboardingViewModel {

    enum Step: Equatable {
        case signIn
        case discoverServers
        case selectLibrary
        case initialSync
    }

    // MARK: - State

    var step: Step = .signIn

    // OAuth
    var isAuthenticating = false
    var oauthError: String?
    var pinCode: String?

    // Server discovery
    var servers: [PlexServer] = []
    var isDiscovering = false
    var selectedServer: PlexServer?
    var isTestingConnections = false
    var connectionType: String?

    // Library
    var musicLibraries: [LibrarySection] = []
    var selectedLibrary: LibrarySection?

    // Sync
    var isSyncing = false
    var syncProgress: SyncEngine.SyncProgress?

    // Security
    var showHTTPWarning = false

    // Server URL fallback
    var showManualURL = false
    var manualURLInput = ""
    var manualURLError: String?

    let plexClient = PlexClient()
    var cache: CacheDatabase?

    // MARK: - OAuth

    func startOAuth() async {
        isAuthenticating = true
        oauthError = nil
        pinCode = nil

        do {
            let pin = try await PlexAuth.createPIN(clientIdentifier: plexClient.clientIdentifier)
            pinCode = pin.code
            let url = PlexAuth.authURL(code: pin.code, clientIdentifier: plexClient.clientIdentifier)
            NSWorkspace.shared.open(url)
            _ = try await PlexAuth.pollForToken(pinID: pin.id, clientIdentifier: plexClient.clientIdentifier)
            isAuthenticating = false
            step = .discoverServers
            await discoverServers()
        } catch {
            oauthError = "Authentication failed: \(error.localizedDescription)"
            isAuthenticating = false
        }
    }

    // MARK: - Server Discovery

    func discoverServers() async {
        guard let token = PlexAuth.storedToken() else {
            oauthError = "No auth token found"
            step = .signIn
            return
        }

        isDiscovering = true
        do {
            servers = try await plexClient.discoverServers(authToken: token)
            if servers.count == 1 {
                await selectServer(servers[0])
            }
        } catch {
            oauthError = "Failed to discover servers: \(error.localizedDescription)"
        }
        isDiscovering = false
    }

    func selectServer(_ server: PlexServer) async {
        selectedServer = server
        isTestingConnections = true
        connectionType = nil

        let refuseHTTP = UserDefaults.standard.bool(forKey: UserDefaultsKeys.refuseHTTP)
        let (bestConnection, isHTTP) = await plexClient.findBestConnection(server: server, allowHTTP: !refuseHTTP)
        if let connection = bestConnection {
            if isHTTP { showHTTPWarning = true }
            connectionType = connection.local ? "Local" : (connection.relay ? "Relay" : "Remote")

            guard let serverURL = URL(string: connection.uri) else {
                isTestingConnections = false
                oauthError = "Invalid server URL"
                return
            }

            do {
                try await plexClient.connect(serverURL: serverURL, token: server.accessToken)
                isTestingConnections = false

                let libs = try await plexClient.findMusicLibraries()
                musicLibraries = libs
                step = .selectLibrary
                if libs.count == 1 {
                    selectLibrary(libs[0])
                }
            } catch {
                isTestingConnections = false
                oauthError = "Failed to connect to \(server.name): \(error.localizedDescription)"
            }
        } else {
            isTestingConnections = false
            if refuseHTTP {
                oauthError = "No secure connection available. Disable \"Refuse HTTP connections\" in Settings or check your network."
            } else {
                oauthError = "Could not reach \(server.name). Try entering the URL manually."
                showManualURL = true
            }
        }
    }

    func connectManualURL() async {
        let trimmed = manualURLInput.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: trimmed), !trimmed.isEmpty else {
            manualURLError = "Invalid URL"
            return
        }

        guard let token = PlexAuth.storedToken() else { return }

        // Security: probe the URL without auth token first to verify it's a real Plex server.
        // This prevents leaking the user's token to arbitrary URLs via social engineering.
        let identityURL = url.appendingPathComponent("identity")
        var probe = URLRequest(url: identityURL, timeoutInterval: 5)
        probe.setValue("application/json", forHTTPHeaderField: "Accept")
        probe.setValue(plexClient.clientIdentifier, forHTTPHeaderField: "X-Plex-Client-Identifier")
        probe.setValue("ramus", forHTTPHeaderField: "X-Plex-Product")
        // Deliberately NO X-Plex-Token — tokenless probe

        do {
            let (data, response) = try await URLSession.shared.data(for: probe)
            guard let http = response as? HTTPURLResponse, (200...299).contains(http.statusCode),
                  let body = String(data: data, encoding: .utf8),
                  body.contains("machineIdentifier") else {
                manualURLError = "That URL doesn't appear to be a Plex server"
                return
            }
        } catch {
            manualURLError = "Could not reach server: \(error.localizedDescription)"
            return
        }

        do {
            try await plexClient.connect(serverURL: url, token: token)
            plexClient.token = token
            manualURLError = nil

            let scheme = url.scheme?.lowercased() ?? "http"
            if scheme == "http" {
                if UserDefaults.standard.bool(forKey: UserDefaultsKeys.refuseHTTP) {
                    manualURLError = "This URL uses unencrypted HTTP. Disable \"Refuse HTTP connections\" in Settings to allow it."
                    return
                }
                showHTTPWarning = true
            }

            // Create a synthetic server config
            selectedServer = PlexServer(
                machineIdentifier: "manual",
                name: trimmed,
                accessToken: token,
                owned: true,
                connections: [PlexServerConnection(
                    uri: trimmed, local: false, relay: false,
                    protocol: scheme
                )]
            )
            connectionType = "Manual"

            let libs = try await plexClient.findMusicLibraries()
            musicLibraries = libs
            step = .selectLibrary
            if libs.count == 1 {
                selectLibrary(libs[0])
            }
        } catch {
            manualURLError = "Connection failed: \(error.localizedDescription)"
        }
    }

    // MARK: - Library

    func selectLibrary(_ lib: LibrarySection) {
        selectedLibrary = lib
        step = .initialSync
    }

    // MARK: - Sync

    func startInitialSync() async {
        guard let library = selectedLibrary else { return }

        do {
            cache = try CacheDatabase()
        } catch {
            oauthError = "Failed to create cache: \(error.localizedDescription)"
            return
        }

        guard let cache else { return }

        isSyncing = true
        let engine = SyncEngine(cache: cache, client: plexClient)

        do {
            try await engine.fullSync(libraryKey: library.key) { [weak self] progress in
                guard let self else { return }
                Task { @MainActor [self] in
                    self.syncProgress = progress
                }
            }
        } catch {
            oauthError = "Sync failed: \(error.localizedDescription)"
        }
        isSyncing = false
    }

    func skipSync() {
        do {
            cache = try CacheDatabase()
        } catch {
            // Non-fatal
        }
    }

    // MARK: - Finalize

    /// Save server config and return configured objects for handoff.
    func finalize() -> ServerConfig? {
        guard let server = selectedServer, let library = selectedLibrary else { return nil }

        let config = ServerConfig(
            machineIdentifier: server.machineIdentifier,
            name: server.name,
            accessToken: server.accessToken,
            selectedLibraryKey: library.key
        )
        PlexAuth.storeServerConfig(config)
        return config
    }
}
