use url::Url;

use crate::models::{PlaybackMode, TranscodeBitrate};
use crate::util::{is_lossless_codec, percent_decode, percent_encode};

/// Whether a track should be transcoded based on playback mode, codec,
/// and connection signals. With mpv, transcoding is only a bandwidth
/// measure — never for codec compatibility. Lossy codecs are always
/// direct-played; the mode only controls whether *lossless* sources get
/// re-encoded.
///
/// `is_cellular` reflects whether the device's primary network interface
/// is cellular — a mobile-only signal that's permanently `false` on
/// desktop.
///
/// This is the *baseline* policy only. It stays a pure function of stable
/// inputs so its test grid can serve as the policy contract; the adaptive
/// response to a link that can't keep up is layered on top of it by the
/// player rather than folded in here.
pub fn should_transcode(codec: Option<&str>, mode: PlaybackMode, is_cellular: bool) -> bool {
    let Some(codec) = codec else { return false };
    if !is_lossless_codec(codec) {
        return false;
    }
    match mode {
        // Both react to a starving link rather than pre-empting it, so
        // neither transcodes on its own.
        PlaybackMode::Never | PlaybackMode::WhenSlow => false,
        PlaybackMode::WhenSlowOrCellular => is_cellular,
        PlaybackMode::Always => true,
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
/// targeting Ogg/Opus at the requested bitrate.
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
///
/// `offset_secs`, when `Some`, starts the transcode partway through the
/// track (used on the connection-failover resume path, never on a fresh
/// first play). The audio transcoder only honours a resume `offset` when
/// it's accompanied by the standard universal-transcode companion params
/// (`mediaIndex`/`partIndex`/`copyts`/`mediaBufferSize`/`protocol`); sent
/// on its own the start request is rejected. Those companions are added
/// only on this path, so the proven minimal `None` call shape is untouched.
pub fn build_transcode_download_url(
    server_url: &Url,
    token: &str,
    track_rating_key: &str,
    client_identifier: &str,
    session: &str,
    bitrate: TranscodeBitrate,
    offset_secs: Option<u64>,
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
    let mut params: Vec<String> = vec![
        "directPlay=0".into(),
        format!("musicBitrate={}", bitrate.as_kbps()),
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

    // Resume offset — only present on the connection-failover reload path.
    // `offset` alone is rejected by the audio transcoder; it needs the
    // companion params below (the shape a universal-transcode client
    // sends). Order is irrelevant to Plex; appended so the normal call
    // shape stays byte-for-byte identical when there's no offset.
    if let Some(secs) = offset_secs {
        params.push(format!("offset={secs}"));
        params.push("mediaIndex=0".into());
        params.push("partIndex=0".into());
        params.push("copyts=1".into());
        params.push("mediaBufferSize=1024".into());
        params.push("protocol=http".into());
    }

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

    const ALL_MODES: [PlaybackMode; 4] = [
        PlaybackMode::Never,
        PlaybackMode::WhenSlow,
        PlaybackMode::WhenSlowOrCellular,
        PlaybackMode::Always,
    ];

    #[test]
    fn test_never_never_transcodes() {
        for is_cellular in [false, true] {
            assert!(!should_transcode(
                Some("flac"),
                PlaybackMode::Never,
                is_cellular
            ));
            assert!(!should_transcode(
                Some("mp3"),
                PlaybackMode::Never,
                is_cellular
            ));
        }
    }

    #[test]
    fn test_when_slow_does_not_transcode_on_its_own() {
        // The baseline policy is direct-play; the adaptive layer is what
        // turns this mode on, and only once the link has been measured
        // unable to sustain the stream.
        for is_cellular in [false, true] {
            assert!(!should_transcode(
                Some("flac"),
                PlaybackMode::WhenSlow,
                is_cellular
            ));
        }
    }

    #[test]
    fn test_always_transcodes_lossless_under_any_flag() {
        for is_cellular in [false, true] {
            for codec in ["flac", "alac", "wav", "aiff", "aif", "pcm"] {
                assert!(
                    should_transcode(Some(codec), PlaybackMode::Always, is_cellular),
                    "{codec} must transcode under Always (is_cellular={is_cellular})"
                );
            }
        }
    }

    #[test]
    fn test_always_does_not_transcode_lossy() {
        for is_cellular in [false, true] {
            for codec in ["mp3", "aac", "opus", "ogg"] {
                assert!(
                    !should_transcode(Some(codec), PlaybackMode::Always, is_cellular),
                    "{codec} is lossy and must never transcode"
                );
            }
        }
    }

    #[test]
    fn test_when_slow_or_cellular_transcodes_on_cellular() {
        assert!(should_transcode(
            Some("flac"),
            PlaybackMode::WhenSlowOrCellular,
            true
        ));
        // Off cellular it falls back to the baseline direct-play; the
        // adaptive layer still covers a slow link, hence the mode name.
        assert!(!should_transcode(
            Some("flac"),
            PlaybackMode::WhenSlowOrCellular,
            false
        ));
    }

    #[test]
    fn test_mode_ladder_is_monotonic() {
        // Each mode must transcode at least everywhere the one above it
        // does — the ordering is what the UI's wording promises.
        for is_cellular in [false, true] {
            let results: Vec<bool> = ALL_MODES
                .iter()
                .map(|m| should_transcode(Some("flac"), *m, is_cellular))
                .collect();
            for pair in results.windows(2) {
                assert!(
                    pair[1] || !pair[0],
                    "ladder regressed at is_cellular={is_cellular}: {results:?}"
                );
            }
        }
    }

    #[test]
    fn test_only_never_refuses_to_adapt() {
        assert!(!PlaybackMode::Never.adapts_to_slow_connection());
        for mode in [
            PlaybackMode::WhenSlow,
            PlaybackMode::WhenSlowOrCellular,
            PlaybackMode::Always,
        ] {
            assert!(mode.adapts_to_slow_connection());
        }
    }

    #[test]
    fn test_lossy_never_transcodes_under_any_mode() {
        for mode in ALL_MODES {
            assert!(!should_transcode(Some("mp3"), mode, true));
            assert!(!should_transcode(Some("aac"), mode, true));
        }
    }

    #[test]
    fn test_none_codec_returns_false() {
        for mode in ALL_MODES {
            assert!(!should_transcode(None, mode, true));
        }
    }

    #[test]
    fn test_transcode_case_insensitive() {
        assert!(should_transcode(Some("FLAC"), PlaybackMode::Always, false));
        assert!(should_transcode(Some("Alac"), PlaybackMode::Always, false));
    }

    #[test]
    fn test_playback_mode_serde_wire_names() {
        // Disk format must match what the frontend sends.
        assert_eq!(
            serde_json::to_string(&PlaybackMode::Never).unwrap(),
            "\"never\""
        );
        assert_eq!(
            serde_json::to_string(&PlaybackMode::WhenSlow).unwrap(),
            "\"whenSlow\""
        );
        assert_eq!(
            serde_json::to_string(&PlaybackMode::WhenSlowOrCellular).unwrap(),
            "\"whenSlowOrCellular\""
        );
        assert_eq!(
            serde_json::to_string(&PlaybackMode::Always).unwrap(),
            "\"always\""
        );
    }

    #[test]
    fn test_retired_wire_names_still_deserialize() {
        // Locality was only ever a proxy for "can't keep up", which is now
        // measured; the standalone cellular mode gained slow-link
        // adaptation. Nothing any of these users had is lost.
        let parse = |s: &str| serde_json::from_str::<PlaybackMode>(s).unwrap();
        assert_eq!(parse("\"remote\""), PlaybackMode::WhenSlow);
        assert_eq!(parse("\"remoteOrCellular\""), PlaybackMode::WhenSlowOrCellular);
        assert_eq!(parse("\"cellular\""), PlaybackMode::WhenSlowOrCellular);
    }

    #[test]
    fn test_transcode_bitrate_serde_wire_names() {
        assert_eq!(
            serde_json::to_string(&TranscodeBitrate::Kbps320).unwrap(),
            "\"kbps320\""
        );
        assert_eq!(
            serde_json::to_string(&TranscodeBitrate::Kbps128).unwrap(),
            "\"kbps128\""
        );
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
            TranscodeBitrate::Kbps128,
            None,
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
        // Match the server-preferred call shape for chunked Opus.
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
    fn test_transcode_download_url_offset_adds_companions() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        // No offset: the minimal proven call shape — none of the resume
        // companions must leak in.
        let plain = build_transcode_download_url(
            &server,
            "t",
            "99251",
            "c",
            "s",
            TranscodeBitrate::Kbps320,
            None,
        )
        .unwrap()
        .to_string();
        assert!(!plain.contains("offset="));
        assert!(!plain.contains("mediaIndex="));
        assert!(!plain.contains("partIndex="));
        assert!(!plain.contains("copyts="));
        assert!(!plain.contains("mediaBufferSize="));
        assert!(!plain.contains("protocol="));

        // With an offset: the resume offset plus every companion the audio
        // transcoder needs for a seekable start.
        let resume = build_transcode_download_url(
            &server,
            "t",
            "99251",
            "c",
            "s",
            TranscodeBitrate::Kbps320,
            Some(137),
        )
        .unwrap()
        .to_string();
        assert!(resume.contains("offset=137"));
        assert!(resume.contains("mediaIndex=0"));
        assert!(resume.contains("partIndex=0"));
        assert!(resume.contains("copyts=1"));
        assert!(resume.contains("mediaBufferSize=1024"));
        assert!(resume.contains("protocol=http"));
        // Companions must not disturb the base call shape.
        assert!(resume.contains("X-Plex-Chunked=1"));
        assert!(resume.contains("musicBitrate=320"));
    }

    #[test]
    fn test_transcode_download_url_carries_opus_profile() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        let url = build_transcode_download_url(
            &server,
            "t",
            "99251",
            "c",
            "s",
            TranscodeBitrate::Kbps128,
            None,
        );
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
        let tx = build_transcode_download_url(
            &server,
            "t",
            "99251",
            "c",
            "s",
            TranscodeBitrate::Kbps128,
            None,
        )
        .unwrap()
        .to_string();
        let direct = build_direct_play_url(&server, "/library/parts/12345/file.flac", "t")
            .unwrap()
            .to_string();
        assert!(is_transcode_download_url(&tx));
        assert!(!is_transcode_download_url(&direct));
    }

    #[test]
    fn test_transcode_url_includes_chosen_bitrate() {
        let server = Url::parse("http://192.168.1.100:32400").unwrap();
        for (bitrate, expected) in [
            (TranscodeBitrate::Kbps320, "musicBitrate=320"),
            (TranscodeBitrate::Kbps256, "musicBitrate=256"),
            (TranscodeBitrate::Kbps192, "musicBitrate=192"),
            (TranscodeBitrate::Kbps128, "musicBitrate=128"),
        ] {
            let url = build_transcode_download_url(&server, "t", "99251", "c", "s", bitrate, None)
                .unwrap()
                .to_string();
            assert!(
                url.contains(expected),
                "URL must contain {expected}; got {url}"
            );
        }
    }
}
