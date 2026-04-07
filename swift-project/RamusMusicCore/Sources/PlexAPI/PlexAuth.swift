import Foundation
import Models
import os.log

private let logger = Logger(subsystem: "com.raspsoft.ramus", category: "PlexAuth")

/// Manages Plex authentication: PIN-based OAuth flow and direct token entry.
/// Stores auth tokens in an encrypted file (AES-GCM, keyed to hardware UUID).
public final class PlexAuth: Sendable {

    // MARK: - Auth Token Storage

    /// Retrieve stored auth token.
    public static func storedToken() -> String? {
        TokenStore.read(.authToken)
    }

    /// Store auth token.
    @discardableResult
    public static func storeToken(_ token: String) -> Bool {
        TokenStore.write(.authToken, value: token)
    }

    /// Delete stored auth token.
    @discardableResult
    public static func deleteToken() -> Bool {
        TokenStore.delete(.authToken)
    }

    // MARK: - Per-Server Token Storage

    @discardableResult
    private static func storeServerToken(_ token: String) -> Bool {
        TokenStore.write(.serverToken, value: token)
    }

    private static func storedServerToken() -> String? {
        TokenStore.read(.serverToken)
    }

    @discardableResult
    private static func deleteServerToken() -> Bool {
        TokenStore.delete(.serverToken)
    }

    // MARK: - PIN-Based OAuth Flow

    /// PIN response from Plex.
    public struct PINResponse: Codable, Sendable {
        public let id: Int64
        public let code: String
        public let authToken: String?
    }

    /// Creates a new Plex PIN for OAuth.
    /// Returns the PIN response containing `id` and `code`.
    public static func createPIN(clientIdentifier: String) async throws -> PINResponse {
        var components = URLComponents(string: "https://plex.tv/api/v2/pins")!
        components.queryItems = [
            URLQueryItem(name: "strong", value: "true"),
            URLQueryItem(name: "X-Plex-Product", value: "ramus"),
            URLQueryItem(name: "X-Plex-Client-Identifier", value: clientIdentifier),
        ]
        var request = URLRequest(url: components.url!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        let (data, response) = try await URLSession.shared.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse,
              (200...299).contains(httpResponse.statusCode) else {
            throw PlexAuthError.pinCreationFailed
        }

        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(PINResponse.self, from: data)
    }

    /// Builds the Plex OAuth URL the user should visit to authorize.
    public static func authURL(code: String, clientIdentifier: String) -> URL {
        URL(string: "https://app.plex.tv/auth#?clientID=\(clientIdentifier)&code=\(code)&context%5Bdevice%5D%5Bproduct%5D=ramus")!
    }

    /// Polls Plex for the auth token after the user has visited the auth URL.
    /// Returns the token once the user authorizes, or throws on timeout.
    public static func pollForToken(pinID: Int64, clientIdentifier: String, maxAttempts: Int = 60, interval: Swift.Duration = .seconds(2)) async throws -> String {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase

        for _ in 0..<maxAttempts {
            var request = URLRequest(url: URL(string: "https://plex.tv/api/v2/pins/\(pinID)")!)
            request.setValue("application/json", forHTTPHeaderField: "Accept")
            request.setValue(clientIdentifier, forHTTPHeaderField: "X-Plex-Client-Identifier")
            request.setValue("ramus", forHTTPHeaderField: "X-Plex-Product")

            let (data, response) = try await URLSession.shared.data(for: request)
            if let http = response as? HTTPURLResponse, http.statusCode == 404 {
                throw PlexAuthError.pinExpired
            }
            let pin = try decoder.decode(PINResponse.self, from: data)

            if let token = pin.authToken, !token.isEmpty {
                if !storeToken(token) {
                    logger.warning("Token write failed — token not persisted. User will need to re-auth next launch.")
                }
                return token
            }

            try await Task.sleep(for: interval)
        }

        throw PlexAuthError.pollingTimeout
    }

    // MARK: - Server Config Persistence

    private static let serverConfigKey = "com.raspsoft.ramus.serverConfig"

    /// Replace stored server config. The access token is stored in the encrypted
    /// token file; non-secret fields go to UserDefaults.
    public static func storeServerConfig(_ config: ServerConfig) {
        storeServerToken(config.accessToken)
        var redacted = config
        redacted.accessToken = ""
        if let data = try? JSONEncoder().encode(redacted) {
            UserDefaults.standard.set(data, forKey: serverConfigKey)
        }
    }

    /// Retrieve stored server config, reconstituting the access token from the token file.
    public static func storedServerConfig() -> ServerConfig? {
        guard let data = UserDefaults.standard.data(forKey: serverConfigKey),
              var config = try? JSONDecoder().decode(ServerConfig.self, from: data) else { return nil }
        if let token = storedServerToken() {
            config.accessToken = token
        }
        return config.accessToken.isEmpty ? nil : config
    }

    /// Delete stored server config and its token.
    public static func deleteServerConfig() {
        deleteServerToken()
        UserDefaults.standard.removeObject(forKey: serverConfigKey)
    }

}

// MARK: - Errors

public enum PlexAuthError: Error, Sendable {
    case pinCreationFailed
    case pollingTimeout
    case pinExpired
}
