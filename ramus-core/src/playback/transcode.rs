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
pub fn build_direct_play_url(server_url: &Url, part_key: &str, token: &str) -> Option<Url> {
    let decoded = percent_decode(part_key);
    if !decoded.starts_with("/library/") || decoded.contains("..") {
        return None;
    }

    let base = server_url.as_str().trim_end_matches('/');
    let url_str = format!("{}{}?X-Plex-Token={}", base, part_key, percent_encode(token));
    Url::parse(&url_str).ok()
}

/// Build a Plex HLS transcode URL against `/music/:/transcode/universal/start.m3u8`.
///
/// The `X-Plex-Client-Profile-Extra` parameter contains pre-encoded values
/// that must not be re-encoded.
pub fn build_hls_url(
    server_url: &Url,
    token: &str,
    track_rating_key: &str,
    client_identifier: &str,
    session: &str,
) -> Option<Url> {
    let base = server_url.as_str().trim_end_matches('/');
    let endpoint = "/music/:/transcode/universal/start.m3u8";

    let params = [
        format!("path=/library/metadata/{}", percent_encode(track_rating_key)),
        "mediaIndex=0".into(),
        "partIndex=0".into(),
        "fastSeek=1".into(),
        "copyts=1".into(),
        "offset=0".into(),
        format!("session={}", percent_encode(session)),
        "directPlay=0".into(),
        "directStreamAudio=0".into(),
        "maxAudioBitrate=256".into(),
        "protocol=hls".into(),
        "X-Plex-Platform=Chrome".into(),
        "X-Plex-Client-Profile-Extra=add-transcode-target(type%3DmusicProfile%26context%3Dstreaming%26protocol%3Dhls%26container%3Dmpegts%26audioCodec%3Daac%2Cmp3)".into(),
        format!("X-Plex-Token={}", percent_encode(token)),
        format!("X-Plex-Client-Identifier={}", percent_encode(client_identifier)),
    ];

    let query = params.join("&");
    Url::parse(&format!("{}{}?{}", base, endpoint, query)).ok()
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
    fn test_hls_url_has_required_parameters() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        let url = build_hls_url(&server, "abc123", "9876", "test-client-id", "fixed-session");
        assert!(url.is_some());
        let url_str = url.unwrap().to_string();
        assert!(url_str.contains("/music/:/transcode/universal/start.m3u8?"));
        assert!(url_str.contains("X-Plex-Token=abc123"));
        assert!(url_str.contains("path=/library/metadata/9876"));
        assert!(url_str.contains("X-Plex-Platform=Chrome"));
        assert!(url_str.contains("protocol=hls"));
    }

    #[test]
    fn test_hls_url_contains_client_profile() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        let url = build_hls_url(&server, "abc123", "9876", "test-client-id", "session");
        assert!(url.is_some());
        let url_str = url.unwrap().to_string();
        assert!(url_str.contains("X-Plex-Client-Profile-Extra="));
        assert!(url_str.contains("musicProfile"));
        assert!(url_str.contains("mpegts"));
    }

    #[test]
    fn test_hls_url_fixed_bitrate() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        let url = build_hls_url(&server, "abc123", "9876", "test-client-id", "session");
        assert!(url.is_some());
        let url_str = url.unwrap().to_string();
        assert!(url_str.contains("maxAudioBitrate=256"));
        assert!(url_str.contains("directPlay=0"));
        assert!(url_str.contains("directStreamAudio=0"));
    }

    #[test]
    fn test_hls_url_correct_endpoint() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        let url = build_hls_url(&server, "token", "123", "client", "session");
        let url_str = url.unwrap().to_string();
        // Must use /music/:/ not /audio/:/
        assert!(url_str.contains("/music/:/transcode/universal/start.m3u8"));
        assert!(!url_str.contains("/audio/:/"));
    }
}
