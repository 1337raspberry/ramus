use url::Url;

use crate::models::PlaybackMode;
use crate::util::{is_lossless_codec, percent_decode, percent_encode};

/// Whether a track should be transcoded based on playback mode, codec,
/// and connection type. With mpv, transcoding is only a bandwidth measure
/// — never for codec compatibility.
pub fn should_transcode(codec: Option<&str>, mode: PlaybackMode, is_remote: bool) -> bool {
    let codec = match codec {
        Some(c) => c,
        None => return false,
    };
    let lossless = is_lossless_codec(codec);

    match mode {
        PlaybackMode::DirectPlay => false,
        PlaybackMode::TranscodeLosslessRemote => lossless && is_remote,
        PlaybackMode::TranscodeLossless => lossless,
    }
}

/// Build a direct-play URL: server base + part key + token as query param.
///
/// Validates that `part_key` starts with `/library/` and contains no path
/// traversal sequences (checked after percent-decoding). Returns `None`
/// on invalid input.
///
/// `download=1` makes the server treat the request as a download rather
/// than a stream: it sets `Content-Disposition: attachment` on the
/// response and surfaces the request on the PMS dashboard as a "Media
/// download by …" entry. Used on every part fetch (player and prefetch).
/// It does NOT raise the per-client concurrency cap.
pub fn build_direct_play_url(server_url: &Url, part_key: &str, token: &str) -> Option<Url> {
    let decoded = percent_decode(part_key);
    if !decoded.starts_with("/library/") || decoded.contains("..") {
        return None;
    }

    let base = server_url.as_str().trim_end_matches('/');
    let url_str = format!(
        "{}{}?download=1&X-Plex-Token={}",
        base,
        part_key,
        percent_encode(token)
    );
    Url::parse(&url_str).ok()
}

/// Build a single-file transcode URL against `/audio/:/transcode/universal/start`,
/// targeting Ogg/Opus at 128 kbps.
///
/// Used by both the live player path (`resolve_url` when `should_transcode`
/// is true) and the prefetch worker. Plex enforces a per-client
/// concurrent-transcode cap of ~1, and a single-file Opus session
/// completes in seconds (mpv slurps the whole 3-5 MB file into its
/// forward buffer at server-transcode speed and then plays from buffer
/// for the song's full duration). Once that session ends, the prefetch
/// worker can fire its own session for the next track without conflict.
/// A previous incarnation of this code used `/music/:/transcode/universal/start.m3u8`
/// (HLS) for live playback, but those sessions stay open in real time
/// for the full song length and got killed the moment the prefetch
/// worker tried to open a second transcode. Single-endpoint solves it.
///
/// `path` carries the **metadata** key (`/library/metadata/<rk>`), not the
/// part key — the server picks the right part itself. Each call should pass
/// a fresh `session` value (the same string is also sent as
/// `X-Plex-Session-Identifier`); the server uses it to dedupe and to GC the
/// ffmpeg process server-side. There is no client-issued `stop?session=…`
/// teardown — abandoned sessions time out on their own.
///
/// The `X-Plex-Client-Profile-Extra` value contains pre-encoded chars and
/// must not be re-encoded.
pub fn build_transcode_download_url(
    server_url: &Url,
    token: &str,
    track_rating_key: &str,
    client_identifier: &str,
    session: &str,
) -> Option<Url> {
    let base = server_url.as_str().trim_end_matches('/');
    let endpoint = "/audio/:/transcode/universal/start";

    // Device value mirrors what `plex::client::PlexClient::device()` would
    // return — server uses it for dashboard labels.
    let device = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "ios") {
        "iOS"
    } else if cfg!(target_os = "android") {
        "Android"
    } else {
        "Linux"
    };

    // Param order, encoding, and identity-param set mirror what other
    // single-file Opus clients send. Notable choices:
    // - `path` is fully percent-encoded (slashes too) — the literal-slash
    //   form works on `/music/:/...m3u8` but the audio transcoder is
    //   stricter and silently fails for some path shapes without it.
    // - `session` is sent as just `<client-id>-<unique-id>` — the server
    //   appears to tokenise on `-` and group sessions by client-id prefix,
    //   so an extra suffix like `-prefetch` makes it conflate distinct
    //   sessions for the same client and quietly drop the second one.
    //   Caller must pass a session value already in `<client-id>-<id>`
    //   shape (we use rating-key as the unique id).
    // - X-Plex-Device-Name / Platform-Version / Version are sent for
    //   parity even though they're informational; Plex's session
    //   bookkeeping seems happier when they're present.
    let params = [
        "directPlay=0".into(),
        "musicBitrate=128".into(),
        format!("path={}", percent_encode("/library/metadata/")) + &percent_encode(track_rating_key),
        format!("session={}", percent_encode(session)),
        "X-Plex-Chunked=1".into(),
        format!("X-Plex-Client-Identifier={}", percent_encode(client_identifier)),
        "X-Plex-Client-Profile-Extra=add-transcode-target(replace%3Dtrue%26type%3DmusicProfile%26context%3Dstreaming%26protocol%3Dhttp%26container%3Dogg%26audioCodec%3Dopus)%2Badd-limitation(scope%3DmusicCodec%26scopeName%3Dopus%26type%3DupperBound%26name%3Daudio%2Echannels%26value%3D2%26onlyTranscodes%3Dtrue%26replace%3Dtrue)".into(),
        format!("X-Plex-Device={device}"),
        format!("X-Plex-Device-Name={}", percent_encode("ramus")),
        // Load-bearing — server picks the transcode profile from
        // X-Plex-Platform. `Generic` pairs with the single-file Ogg/Opus
        // output target above; without it the server can't match the
        // requested profile and rejects the request.
        "X-Plex-Platform=Generic".into(),
        format!("X-Plex-Platform-Version={}", percent_encode(std::env::consts::OS)),
        "X-Plex-Product=ramus".into(),
        format!("X-Plex-Session-Identifier={}", percent_encode(session)),
        format!("X-Plex-Token={}", percent_encode(token)),
        format!("X-Plex-Version={}", percent_encode(env!("CARGO_PKG_VERSION"))),
    ];

    let query = params.join("&");
    Url::parse(&format!("{}{}?{}", base, endpoint, query)).ok()
}

/// Returns true if `url` is a transcode-download URL (the kind built by
/// `build_transcode_download_url`). Used by the prefetch worker to pick the
/// right on-disk file extension for the cached output, since the URL has no
/// extension to derive one from.
pub fn is_transcode_download_url(url: &str) -> bool {
    url.contains("/audio/:/transcode/universal/start")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direct_play_never_transcodes() {
        assert!(!should_transcode(Some("flac"), PlaybackMode::DirectPlay, false));
        assert!(!should_transcode(Some("flac"), PlaybackMode::DirectPlay, true));
        assert!(!should_transcode(Some("mp3"), PlaybackMode::DirectPlay, false));
    }

    #[test]
    fn test_transcode_lossless_transcodes_lossless() {
        assert!(should_transcode(Some("flac"), PlaybackMode::TranscodeLossless, false));
        assert!(should_transcode(Some("flac"), PlaybackMode::TranscodeLossless, true));
        assert!(should_transcode(Some("alac"), PlaybackMode::TranscodeLossless, false));
        assert!(should_transcode(Some("wav"), PlaybackMode::TranscodeLossless, false));
        assert!(should_transcode(Some("aiff"), PlaybackMode::TranscodeLossless, false));
        assert!(should_transcode(Some("aif"), PlaybackMode::TranscodeLossless, false));
        assert!(should_transcode(Some("pcm"), PlaybackMode::TranscodeLossless, false));
    }

    #[test]
    fn test_transcode_lossless_does_not_transcode_lossy() {
        assert!(!should_transcode(Some("mp3"), PlaybackMode::TranscodeLossless, false));
        assert!(!should_transcode(Some("aac"), PlaybackMode::TranscodeLossless, false));
        assert!(!should_transcode(Some("opus"), PlaybackMode::TranscodeLossless, false));
        assert!(!should_transcode(Some("ogg"), PlaybackMode::TranscodeLossless, false));
    }

    #[test]
    fn test_transcode_lossless_remote_only_when_remote() {
        assert!(should_transcode(
            Some("flac"),
            PlaybackMode::TranscodeLosslessRemote,
            true
        ));
        assert!(!should_transcode(
            Some("flac"),
            PlaybackMode::TranscodeLosslessRemote,
            false
        ));
    }

    #[test]
    fn test_transcode_lossless_remote_does_not_transcode_lossy() {
        assert!(!should_transcode(
            Some("mp3"),
            PlaybackMode::TranscodeLosslessRemote,
            true
        ));
    }

    #[test]
    fn test_transcode_none_codec_returns_false() {
        assert!(!should_transcode(None, PlaybackMode::TranscodeLossless, false));
        assert!(!should_transcode(None, PlaybackMode::TranscodeLosslessRemote, true));
    }

    #[test]
    fn test_transcode_case_insensitive() {
        assert!(should_transcode(Some("FLAC"), PlaybackMode::TranscodeLossless, false));
        assert!(should_transcode(Some("Alac"), PlaybackMode::TranscodeLossless, false));
    }

    #[test]
    fn test_direct_play_url_includes_token() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        let url = build_direct_play_url(&server, "/library/parts/12345/file.flac", "abc123");
        assert!(url.is_some());
        let url_str = url.unwrap().to_string();
        assert!(url_str.contains("X-Plex-Token=abc123"));
        assert!(url_str.contains("/library/parts/12345/file.flac"));
    }

    #[test]
    fn test_direct_play_url_includes_download_flag() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        let url = build_direct_play_url(&server, "/library/parts/12345/file.flac", "abc123");
        let url_str = url.unwrap().to_string();
        assert!(
            url_str.contains("download=1"),
            "URL must carry download=1 for PMS dashboard tracking; got {url_str}"
        );
    }

    #[test]
    fn test_direct_play_url_rejects_non_library_path() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        assert!(build_direct_play_url(&server, "/etc/passwd", "token").is_none());
        assert!(build_direct_play_url(&server, "/other/path", "token").is_none());
    }

    #[test]
    fn test_direct_play_url_rejects_path_traversal() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        assert!(build_direct_play_url(&server, "/library/../etc/passwd", "token").is_none());
        assert!(build_direct_play_url(&server, "/library/parts/../../secret", "token").is_none());
    }

    #[test]
    fn test_direct_play_url_rejects_encoded_traversal() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        assert!(
            build_direct_play_url(&server, "/library/%2e%2e/etc/passwd", "token").is_none()
        );
    }

    #[test]
    fn test_transcode_download_url_endpoint_and_params() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        let url = build_transcode_download_url(
            &server,
            "abc123",
            "99251",
            "test-client-id",
            "test-client-id-99251",
        );
        let url_str = url.unwrap().to_string();
        // Endpoint must be /audio/:/, no .m3u8 — distinct from the
        // (now-retired) /music/:/...m3u8 HLS endpoint.
        assert!(url_str.contains("/audio/:/transcode/universal/start?"));
        assert!(!url_str.contains("/music/:/"));
        assert!(!url_str.contains(".m3u8"));
        // path param uses metadata key, not part key, fully URL-encoded.
        assert!(url_str.contains("path=%2Flibrary%2Fmetadata%2F99251"));
        assert!(!url_str.contains("path=/library"));
        // Match Plexamp's call shape.
        assert!(url_str.contains("directPlay=0"));
        assert!(url_str.contains("musicBitrate=128"));
        assert!(url_str.contains("X-Plex-Chunked=1"));
        assert!(url_str.contains("session=test-client-id-99251"));
        assert!(url_str.contains("X-Plex-Session-Identifier=test-client-id-99251"));
        assert!(url_str.contains("X-Plex-Token=abc123"));
        assert!(url_str.contains("X-Plex-Client-Identifier=test-client-id"));
        // Identity params — server uses Platform=Generic to pick the
        // single-file Ogg/Opus profile.
        assert!(url_str.contains("X-Plex-Platform=Generic"));
        assert!(url_str.contains("X-Plex-Product=ramus"));
        assert!(url_str.contains("X-Plex-Device="));
        assert!(url_str.contains("X-Plex-Device-Name="));
        assert!(url_str.contains("X-Plex-Platform-Version="));
        assert!(url_str.contains("X-Plex-Version="));
    }

    #[test]
    fn test_transcode_download_url_carries_opus_profile() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        let url = build_transcode_download_url(&server, "t", "99251", "c", "s");
        let url_str = url.unwrap().to_string();
        // The Opus / Ogg target must survive into the final URL — these
        // pre-encoded chars are what tells the server "give me an Opus stream".
        assert!(url_str.contains("X-Plex-Client-Profile-Extra="));
        assert!(url_str.contains("musicProfile"));
        assert!(url_str.contains("audioCodec%3Dopus"));
        assert!(url_str.contains("container%3Dogg"));
    }

    #[test]
    fn test_is_transcode_download_url() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        let tx = build_transcode_download_url(&server, "t", "99251", "c", "s")
            .unwrap()
            .to_string();
        let direct = build_direct_play_url(&server, "/library/parts/12345/file.flac", "t")
            .unwrap()
            .to_string();
        assert!(is_transcode_download_url(&tx));
        assert!(!is_transcode_download_url(&direct));
    }
}
