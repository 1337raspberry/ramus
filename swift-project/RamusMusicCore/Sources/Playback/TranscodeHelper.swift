import Foundation

/// URL construction for Plex audio playback.
public enum TranscodeHelper: Sendable {

    /// Build a direct-play URL: server base + part key + token as query param.
    public static func buildDirectPlayURL(serverURL: URL, partKey: String, token: String) -> URL? {
        // Percent-decode before checking for traversal sequences — `appendingPathComponent`
        // decodes %2e%2e/.%2e variants, so the guard must see the decoded form.
        let decoded = partKey.removingPercentEncoding ?? partKey
        guard decoded.hasPrefix("/library/"), !decoded.contains("..") else { return nil }
        var components = URLComponents(
            url: serverURL.appendingPathComponent(partKey),
            resolvingAgainstBaseURL: false
        )
        components?.queryItems = [URLQueryItem(name: "X-Plex-Token", value: token)]
        return components?.url
    }

    /// Build a Plex HLS transcode URL.
    /// Uses `/music/:/transcode/universal/start.m3u8` — the audio HLS endpoint
    /// that returns a VOD manifest with 1-second MP3-in-MPEGTS segments.
    /// Requires `X-Plex-Client-Profile-Extra` and `X-Plex-Platform=Chrome`.
    public static func buildHLSURL(
        serverURL: URL,
        token: String,
        trackRatingKey: String,
        clientIdentifier: String,
        session: String = UUID().uuidString
    ) -> URL? {
        // Build URL via string to avoid double-encoding of pre-encoded profile value.
        // The profile parameter contains %3D/%26/%2C that must NOT be re-encoded.
        // Individual values are percent-encoded for defense in depth.
        let endpoint = "/music/:/transcode/universal/start.m3u8"

        let queryValueAllowed: CharacterSet = {
            var cs = CharacterSet.urlQueryAllowed
            cs.remove(charactersIn: "&=#+")
            return cs
        }()
        let encode = { (s: String) in s.addingPercentEncoding(withAllowedCharacters: queryValueAllowed) ?? s }
        let params = [
            "path=/library/metadata/\(encode(trackRatingKey))",
            "mediaIndex=0",
            "partIndex=0",
            "fastSeek=1",
            "copyts=1",
            "offset=0",
            "session=\(encode(session))",
            "directPlay=0",
            "directStreamAudio=0",
            "maxAudioBitrate=256",
            "protocol=hls",
            "X-Plex-Platform=Chrome",
            "X-Plex-Client-Profile-Extra=add-transcode-target(type%3DmusicProfile%26context%3Dstreaming%26protocol%3Dhls%26container%3Dmpegts%26audioCodec%3Daac%2Cmp3)",
            "X-Plex-Token=\(encode(token))",
            "X-Plex-Client-Identifier=\(encode(clientIdentifier))",
        ]
        let query = params.joined(separator: "&")
        let base = serverURL.absoluteString
        return URL(string: "\(base)\(endpoint)?\(query)")
    }
}
