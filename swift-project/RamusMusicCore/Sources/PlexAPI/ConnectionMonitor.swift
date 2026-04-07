import Foundation
import Network
import Models
import os

private let log = Logger(subsystem: "com.raspsoft.ramus", category: "ConnectionMonitor")

/// Monitors network path changes and automatically re-evaluates the best
/// Plex server connection. Debounces rapid path changes (VPN toggles, etc.)
/// and falls back through cached connections before attempting re-discovery.
///
/// All mutable state is protected by actor isolation.
public actor ConnectionMonitor {

    private let client: PlexClient
    private let monitor: NWPathMonitor
    private let monitorQueue = DispatchQueue(label: "com.raspsoft.ramus.networkMonitor")

    // State — all protected by actor isolation
    private var cachedServer: PlexServer?
    private var activeURI: String?
    private var authToken: String?
    private var isEvaluating = false
    private var debounceTask: Task<Void, Never>?
    private var lastInterfaceNames: Set<String> = []
    private var allowHTTP: Bool = true

    /// Fires when a new working connection is found. Parameters: (serverURL, serverAccessToken, isLocal, isHTTP).
    private var onConnectionChanged: (@Sendable (URL, String, Bool, Bool) async -> Void)?

    /// Fires when all connections (cached + re-discovery) fail.
    private var onConnectionLost: (@Sendable () async -> Void)?

    public init(client: PlexClient) {
        self.client = client
        self.monitor = NWPathMonitor()
    }

    public func setOnConnectionChanged(_ handler: @escaping @Sendable (URL, String, Bool, Bool) async -> Void) {
        onConnectionChanged = handler
    }

    public func setAllowHTTP(_ value: Bool) {
        allowHTTP = value
    }

    public func setOnConnectionLost(_ handler: @escaping @Sendable () async -> Void) {
        onConnectionLost = handler
    }

    // MARK: - Lifecycle

    public func start(server: PlexServer, activeConnectionURI: String, authToken: String) {
        self.cachedServer = server
        self.activeURI = activeConnectionURI
        self.authToken = authToken

        log.info("started monitoring — active: \(activeConnectionURI, privacy: .private), \(server.sortedConnections.count, privacy: .public) cached connections")

        // NWPathMonitor fires on monitorQueue — capture values there, dispatch to actor
        monitor.pathUpdateHandler = { [weak self] path in
            guard path.status == .satisfied, let self else { return }
            let interfaces = Set(path.availableInterfaces.map(\.name))
            Task { await self.handlePathUpdate(interfaces: interfaces) }
        }
        monitor.start(queue: monitorQueue)
    }

    public func stop() {
        monitor.cancel()
        debounceTask?.cancel()
    }

    // MARK: - Path Handling

    private func handlePathUpdate(interfaces: Set<String>) {
        // Skip spurious events where the interface set hasn't actually changed
        guard interfaces != lastInterfaceNames else { return }
        let added = interfaces.subtracting(lastInterfaceNames)
        let removed = lastInterfaceNames.subtracting(interfaces)
        log.info("path update: interfaces changed — added: \(added.sorted(), privacy: .public), removed: \(removed.sorted(), privacy: .public). Evaluating in 500ms...")
        lastInterfaceNames = interfaces

        debounceTask?.cancel()
        debounceTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(500))
            guard !Task.isCancelled else { return }
            await self?.evaluateConnection()
        }
    }

    // MARK: - Connection Evaluation

    /// Test current connection and fall back through alternatives if it fails.
    /// Called automatically on network changes and can be called manually (e.g. from retry logic).
    public func evaluateConnection() async {
        // Actor isolation makes this check-and-set atomic — no suspension point between
        // the guard and the assignment, so no TOCTOU race is possible.
        guard !isEvaluating else { return }
        isEvaluating = true
        defer { isEvaluating = false }
        guard let server = cachedServer, let currentURI = activeURI else { return }

        log.info("evaluating connection (current: \(currentURI, privacy: .private))")

        // Fast path: current connection still works (and satisfies HTTP policy)
        let currentIsHTTP = currentURI.hasPrefix("http://")
        if !currentIsHTTP || allowHTTP,
           await client.testConnection(uri: currentURI, token: server.accessToken, timeout: 3) {
            log.info("current connection OK")
            return
        }
        if currentIsHTTP, !allowHTTP {
            log.warning("current connection is HTTP but policy requires HTTPS — searching for secure alternative")
        } else {
            log.warning("current connection failed — trying \(server.sortedConnections.count, privacy: .public) cached alternatives")
        }

        // Try all cached connections in priority order
        for connection in server.sortedConnections where connection.uri != currentURI {
            guard allowHTTP || connection.protocol == "https" else { continue }
            let local = connection.local ? "local" : "remote"
            if await client.testConnection(uri: connection.uri, token: server.accessToken, timeout: 5) {
                let isHTTP = connection.protocol == "http"
                log.info("switched to \(local, privacy: .public) connection: \(connection.uri, privacy: .private)")
                activeURI = connection.uri
                if let url = URL(string: connection.uri) {
                    await onConnectionChanged?(url, server.accessToken, connection.local, isHTTP)
                }
                return
            } else {
                log.info("  \(local, privacy: .public) \(connection.uri, privacy: .private) — failed")
            }
        }

        // Last resort: re-discover from plex.tv
        log.warning("all cached connections failed — re-discovering from plex.tv")
        guard let authToken else {
            log.error("no auth token for re-discovery")
            await onConnectionLost?()
            return
        }

        do {
            let servers = try await client.discoverServers(authToken: authToken)
            guard let freshServer = servers.first(where: { $0.machineIdentifier == server.machineIdentifier }) else {
                log.error("re-discovery failed — server not found")
                await onConnectionLost?()
                return
            }
            let (connection, isHTTP) = await client.findBestConnection(server: freshServer, allowHTTP: allowHTTP)
            guard let connection, let url = URL(string: connection.uri) else {
                log.error("re-discovery failed — no working connection found")
                await onConnectionLost?()
                return
            }

            let local = connection.local ? "local" : "remote"
            log.info("re-discovery succeeded — switched to \(local, privacy: .public): \(connection.uri, privacy: .private)")
            cachedServer = freshServer
            activeURI = connection.uri
            await onConnectionChanged?(url, freshServer.accessToken, connection.local, isHTTP)
        } catch {
            log.error("re-discovery error: \(error.localizedDescription, privacy: .public)")
            await onConnectionLost?()
        }
    }
}
