//! Lyrics fetching and LRC format parsing for Plex and LRCLIB sources.

/// A single line of lyrics.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub id: usize,
    /// Timestamp in seconds. `None` for unsynced lyrics.
    pub timestamp: Option<f64>,
    pub text: String,
}

/// Source of lyrics data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LyricsSource {
    Plex,
    Lrclib,
}

/// Parsed lyrics result with sync state and source.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsResult {
    pub lines: Vec<LyricLine>,
    pub is_synced: bool,
    pub source: LyricsSource,
}

impl LyricsResult {
    /// Index of the active lyric line at the given playback position.
    ///
    /// Returns the last line whose timestamp <= position. Returns `None` if
    /// no synced lines exist or position precedes the first line.
    pub fn active_line_index(&self, position: f64) -> Option<usize> {
        if !self.is_synced {
            return None;
        }

        let synced: Vec<(usize, f64)> = self
            .lines
            .iter()
            .enumerate()
            .filter_map(|(i, line)| line.timestamp.map(|ts| (i, ts)))
            .collect();

        if synced.is_empty() {
            return None;
        }

        let mut result: Option<usize> = None;
        let mut lo = 0usize;
        let mut hi = synced.len();

        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if synced[mid].1 <= position {
                result = Some(synced[mid].0);
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        result
    }
}

/// Outcome of a lyrics fetch attempt.
///
/// Distinguishes a definitive negative from a retryable transient failure so
/// callers can retry only the latter and report an honest status to the user
/// instead of collapsing every outcome into "not found".
#[derive(Debug, Clone, PartialEq)]
pub enum LyricsOutcome {
    /// Lyrics were found.
    Found(LyricsResult),
    /// The source has definitively no lyrics for this track (e.g. LRCLIB 404,
    /// or a 2xx response carrying no usable lyrics). Retrying won't help.
    NotFound,
    /// A transient failure — request timeout, network error, or a 5xx/429
    /// response. Worth retrying; the source may answer on a later attempt.
    Transient,
}

/// IPC-facing lyrics fetch status.
///
/// Mirrors [`LyricsOutcome`] but splits the transient case into "device
/// offline" vs "source unreachable" so the UI can show an honest message
/// instead of a blanket "not found". Serialized camelCase to match the TS
/// union (`found` | `notFound` | `offline` | `unreachable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LyricsStatus {
    /// Lyrics were found (`lyrics` is populated on the response).
    Found,
    /// A source definitively has no lyrics for this track.
    NotFound,
    /// The device has no internet connection.
    Offline,
    /// The device is online but no lyrics source could be reached.
    Unreachable,
}

/// IPC response for `fetch_lyrics`: an honest status plus the lyrics when found.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsFetchResult {
    pub status: LyricsStatus,
    pub lyrics: Option<LyricsResult>,
}

/// Parse LRC format lyrics text.
///
/// Format: `[MM:SS.cc] text` where cc is centiseconds.
/// Lines without valid timestamps or with empty text are skipped.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    let mut id = 0;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(parsed) = parse_lrc_line(trimmed, id) {
            lines.push(parsed);
            id += 1;
        }
    }

    lines
}

fn parse_lrc_line(line: &str, id: usize) -> Option<LyricLine> {
    if !line.starts_with('[') {
        return None;
    }

    let bracket_end = line.find(']')?;
    let timestamp_str = &line[1..bracket_end];
    let text = line[bracket_end + 1..].trim().to_string();

    if text.is_empty() {
        return None;
    }

    let timestamp = parse_lrc_timestamp(timestamp_str)?;

    Some(LyricLine {
        id,
        timestamp: Some(timestamp),
        text,
    })
}

fn parse_lrc_timestamp(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return None;
    }

    let minutes: f64 = parts[0].parse().ok()?;
    let seconds: f64 = parts[1].parse().ok()?;

    Some(minutes * 60.0 + seconds)
}

/// Parse plain text lyrics (one line per line, no timestamps).
pub fn parse_plain_lyrics(text: &str) -> Vec<LyricLine> {
    let mut id = 0;
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let lyric = LyricLine {
                id,
                timestamp: None,
                text: trimmed.to_string(),
            };
            id += 1;
            Some(lyric)
        })
        .collect()
}

/// LRCLIB API response.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LrclibResponse {
    pub synced_lyrics: Option<String>,
    pub plain_lyrics: Option<String>,
}

/// Plex lyrics response: `MediaContainer > Lyrics > Line > Span`.
#[derive(Debug, serde::Deserialize)]
struct PlexLyricsResponse {
    #[serde(rename = "MediaContainer")]
    media_container: PlexLyricsContainer,
}

#[derive(Debug, serde::Deserialize)]
struct PlexLyricsContainer {
    #[serde(rename = "Lyrics")]
    lyrics: Option<Vec<PlexLyric>>,
}

#[derive(Debug, serde::Deserialize)]
struct PlexLyric {
    #[serde(rename = "Line")]
    line: Option<Vec<PlexLyricLine>>,
}

#[derive(Debug, serde::Deserialize)]
struct PlexLyricLine {
    #[serde(rename = "Span")]
    span: Option<Vec<PlexLyricSpan>>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlexLyricSpan {
    text: Option<String>,
    /// Milliseconds.
    start_offset: Option<i64>,
}

/// Parse Plex JSON lyrics format (`MediaContainer > Lyrics > Line > Span`).
///
/// Each line's text is the concatenation of its span texts. Timestamp comes
/// from the first span's `startOffset` in milliseconds, converted to seconds.
pub fn parse_plex_json_lyrics(data: &[u8]) -> Option<Vec<LyricLine>> {
    let response: PlexLyricsResponse = serde_json::from_slice(data).ok()?;
    let lyrics = response.media_container.lyrics?;
    let lyric = lyrics.into_iter().next()?;
    let lines = lyric.line?;

    let mut result = Vec::new();
    for (id, line) in lines.iter().enumerate() {
        let spans = match &line.span {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };

        let text: String = spans
            .iter()
            .filter_map(|s| s.text.as_deref())
            .collect::<Vec<_>>()
            .join("");

        if text.trim().is_empty() {
            continue;
        }

        let timestamp = spans[0].start_offset.map(|ms| ms as f64 / 1000.0);

        result.push(LyricLine {
            id,
            timestamp,
            text,
        });
    }

    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Validate a Plex lyrics stream path: must start with `/library/` or
/// `/file/`, and must not contain path traversal.
pub fn validate_lyrics_path(path: &str) -> bool {
    let decoded = crate::util::percent_decode(path);
    (decoded.starts_with("/library/") || decoded.starts_with("/file/")) && !decoded.contains("..")
}

/// Maximum LRCLIB response size (512 KB).
const LRCLIB_MAX_RESPONSE: usize = 512 * 1024;

/// LRCLIB base URL. Split out so tests can target a mock server.
const LRCLIB_BASE_URL: &str = "https://lrclib.net";

/// Per-request LRCLIB timeout. LRCLIB's backend routinely takes 6-12s to
/// answer a `/api/get` (occasionally longer, and rarely a 504), so this must
/// comfortably exceed that window or we time out responses that were about to
/// succeed. The retry count and the command-level overall budget bound the
/// worst case.
const LRCLIB_TIMEOUT_SECS: u64 = 15;

/// Client identifier sent to LRCLIB (and reused as the User-Agent).
const LRCLIB_CLIENT_TAG: &str = concat!(
    "ramus v",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/1337raspberry/ramus)",
);

/// Fetch lyrics from LRCLIB. Tries synced (LRC) first, falls back to plain text.
///
/// Returns a [`LyricsOutcome`] so the caller can tell a definitive "no lyrics"
/// (HTTP 404, or a 2xx body with nothing usable) from a transient failure
/// (network error, timeout, 5xx/429) and retry only the latter.
pub async fn fetch_from_lrclib(
    http: &reqwest::Client,
    track_name: &str,
    artist_name: &str,
    album_name: &str,
    duration_secs: f64,
) -> LyricsOutcome {
    fetch_from_lrclib_at(
        http,
        LRCLIB_BASE_URL,
        track_name,
        artist_name,
        album_name,
        duration_secs,
    )
    .await
}

async fn fetch_from_lrclib_at(
    http: &reqwest::Client,
    base_url: &str,
    track_name: &str,
    artist_name: &str,
    album_name: &str,
    duration_secs: f64,
) -> LyricsOutcome {
    let duration_int = duration_secs as u64;
    // Map any reqwest error to `Transient` at this boundary and never carry it
    // upward: `reqwest::Error`'s Display leaks the request URL.
    let resp = match http
        .get(format!("{base_url}/api/get"))
        .query(&[
            ("track_name", track_name),
            ("artist_name", artist_name),
            ("album_name", album_name),
            ("duration", &duration_int.to_string()),
        ])
        .header("Lrclib-Client", LRCLIB_CLIENT_TAG)
        .header("User-Agent", LRCLIB_CLIENT_TAG)
        .timeout(std::time::Duration::from_secs(LRCLIB_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(_) => return LyricsOutcome::Transient,
    };

    let status = resp.status();
    if !status.is_success() {
        // Server errors and rate-limits may clear up; a 404 (or any other
        // client error like a malformed query) is definitive.
        if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return LyricsOutcome::Transient;
        }
        return LyricsOutcome::NotFound;
    }

    let body = match resp.bytes().await {
        Ok(body) => body,
        Err(_) => return LyricsOutcome::Transient,
    };
    // A structurally bad 2xx response won't improve on retry — treat as
    // definitively unusable rather than transient.
    if body.len() > LRCLIB_MAX_RESPONSE {
        return LyricsOutcome::NotFound;
    }

    let parsed: LrclibResponse = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(_) => return LyricsOutcome::NotFound,
    };

    if let Some(synced) = parsed.synced_lyrics {
        let lines = parse_lrc(&synced);
        if !lines.is_empty() {
            return LyricsOutcome::Found(LyricsResult {
                is_synced: true,
                lines,
                source: LyricsSource::Lrclib,
            });
        }
    }

    if let Some(plain) = parsed.plain_lyrics {
        let lines = parse_plain_lyrics(&plain);
        if !lines.is_empty() {
            return LyricsOutcome::Found(LyricsResult {
                is_synced: false,
                lines,
                source: LyricsSource::Lrclib,
            });
        }
    }

    LyricsOutcome::NotFound
}

/// Fetch lyrics from the track's Plex lyrics stream, if it has one.
///
/// Returns the parsed lyrics on success and `None` for everything else —
/// absence of a lyrics stream, an invalid path, or any fetch/parse failure.
/// Plex is a preferred-but-optional source: a failure here simply falls
/// through to LRCLIB, so it deliberately does not surface transient errors.
pub async fn fetch_from_plex(
    plex: &crate::plex::client::PlexClient,
    rating_key: &str,
) -> Option<LyricsResult> {
    let stream = plex.fetch_lyrics_stream(rating_key).await.ok()??;
    let key = stream.key.as_deref()?;
    if !validate_lyrics_path(key) {
        return None;
    }
    let data = plex.download_lyrics_data(key).await.ok()?;
    let lines = if key.ends_with(".lrc") {
        parse_lrc(&String::from_utf8_lossy(&data))
    } else {
        parse_plex_json_lyrics(&data)?
    };
    if lines.is_empty() {
        return None;
    }
    let is_synced = lines.iter().any(|l| l.timestamp.is_some());
    Some(LyricsResult {
        lines,
        is_synced,
        source: LyricsSource::Plex,
    })
}

/// Number of LRCLIB attempts before giving up with a transient status. A
/// second attempt only helps a fast-failing transient (a quick 5xx/429); a
/// merely-slow LRCLIB won't answer faster on retry, so we don't pile on more
/// (and the command's overall budget caps the total wait regardless).
const LRCLIB_MAX_ATTEMPTS: u32 = 2;

/// Fetch lyrics for a track, preferring the user's Plex server and falling
/// back to LRCLIB.
///
/// Plex is tried once: a transient Plex blip must not add retry latency
/// (a remote server that's simply down would otherwise stall every track) nor
/// mask LRCLIB's authoritative answer. LRCLIB — the comprehensive crowdsourced
/// source — is retried a few times on transient failures with a short backoff.
/// The returned [`LyricsOutcome`] lets the caller report an honest status.
pub async fn fetch_lyrics_full(
    plex: &crate::plex::client::PlexClient,
    http: &reqwest::Client,
    rating_key: &str,
    title: &str,
    artist: &str,
    album: &str,
    duration: f64,
) -> LyricsOutcome {
    if let Some(result) = fetch_from_plex(plex, rating_key).await {
        return LyricsOutcome::Found(result);
    }

    let mut delay = std::time::Duration::from_millis(200);
    for attempt in 0..LRCLIB_MAX_ATTEMPTS {
        match fetch_from_lrclib(http, title, artist, album, duration).await {
            LyricsOutcome::Found(result) => return LyricsOutcome::Found(result),
            LyricsOutcome::NotFound => return LyricsOutcome::NotFound,
            LyricsOutcome::Transient => {
                if attempt + 1 < LRCLIB_MAX_ATTEMPTS {
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
            }
        }
    }
    LyricsOutcome::Transient
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_lrc_basic() {
        let lrc = "[00:12.34] Hello world\n[00:15.00] Second line";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello world");
        assert!((lines[0].timestamp.unwrap() - 12.34).abs() < 0.01);
        assert_eq!(lines[1].text, "Second line");
        assert!((lines[1].timestamp.unwrap() - 15.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_lrc_minutes() {
        let lrc = "[02:30.00] Two minutes thirty";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 1);
        assert!((lines[0].timestamp.unwrap() - 150.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_lrc_centiseconds() {
        let lrc = "[01:23.45] Precise timing";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 1);
        assert!((lines[0].timestamp.unwrap() - 83.45).abs() < 0.01);
    }

    #[test]
    fn test_parse_lrc_skips_empty_text() {
        let lrc = "[00:00.00] \n[00:05.00] Real line";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Real line");
    }

    #[test]
    fn test_parse_lrc_skips_invalid_lines() {
        let lrc = "Not a timestamp\n[00:05.00] Valid line\nAnother invalid";
        let lines = parse_lrc(lrc);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Valid line");
    }

    #[test]
    fn test_parse_lrc_empty_input() {
        assert!(parse_lrc("").is_empty());
    }

    #[test]
    fn test_parse_lrc_sequential_ids() {
        let lrc = "[00:01.00] First\n[00:02.00] Second\n[00:03.00] Third";
        let lines = parse_lrc(lrc);
        assert_eq!(lines[0].id, 0);
        assert_eq!(lines[1].id, 1);
        assert_eq!(lines[2].id, 2);
    }

    #[test]
    fn test_parse_plain_lyrics() {
        let text = "Line one\nLine two\n\nLine four";
        let lines = parse_plain_lyrics(text);
        assert_eq!(lines.len(), 3);
        assert!(lines.iter().all(|l| l.timestamp.is_none()));
        assert_eq!(lines[0].text, "Line one");
        assert_eq!(lines[1].text, "Line two");
        assert_eq!(lines[2].text, "Line four");
    }

    #[test]
    fn test_parse_plain_lyrics_trims_whitespace() {
        let text = "  padded  \n\ttabbed\t";
        let lines = parse_plain_lyrics(text);
        assert_eq!(lines[0].text, "padded");
        assert_eq!(lines[1].text, "tabbed");
    }

    #[test]
    fn test_parse_plain_lyrics_sequential_ids() {
        let text = "A\n\nB\nC";
        let lines = parse_plain_lyrics(text);
        assert_eq!(lines[0].id, 0);
        assert_eq!(lines[1].id, 1);
        assert_eq!(lines[2].id, 2);
    }

    #[test]
    fn test_active_line_index_synced() {
        let result = LyricsResult {
            lines: vec![
                LyricLine { id: 0, timestamp: Some(0.0), text: "First".into() },
                LyricLine { id: 1, timestamp: Some(5.0), text: "Second".into() },
                LyricLine { id: 2, timestamp: Some(10.0), text: "Third".into() },
                LyricLine { id: 3, timestamp: Some(15.0), text: "Fourth".into() },
            ],
            is_synced: true,
            source: LyricsSource::Lrclib,
        };

        assert_eq!(result.active_line_index(-1.0), None);
        assert_eq!(result.active_line_index(0.0), Some(0));
        assert_eq!(result.active_line_index(3.0), Some(0));
        assert_eq!(result.active_line_index(5.0), Some(1));
        assert_eq!(result.active_line_index(7.5), Some(1));
        assert_eq!(result.active_line_index(10.0), Some(2));
        assert_eq!(result.active_line_index(100.0), Some(3));
    }

    #[test]
    fn test_active_line_index_unsynced_returns_none() {
        let result = LyricsResult {
            lines: vec![LyricLine { id: 0, timestamp: None, text: "Line".into() }],
            is_synced: false,
            source: LyricsSource::Plex,
        };
        assert_eq!(result.active_line_index(5.0), None);
    }

    #[test]
    fn test_active_line_index_empty_returns_none() {
        let result = LyricsResult {
            lines: vec![],
            is_synced: true,
            source: LyricsSource::Lrclib,
        };
        assert_eq!(result.active_line_index(5.0), None);
    }

    #[test]
    fn test_is_synced_detection() {
        let synced_lines = parse_lrc("[00:01.00] Synced");
        assert!(synced_lines.iter().any(|l| l.timestamp.is_some()));

        let plain_lines = parse_plain_lyrics("Plain");
        assert!(!plain_lines.iter().any(|l| l.timestamp.is_some()));
    }

    #[test]
    fn test_lrclib_response_deserialization() {
        let json = r#"{"syncedLyrics":"[00:01.00] Hi","plainLyrics":"Hi"}"#;
        let resp: LrclibResponse = serde_json::from_str(json).unwrap();
        assert!(resp.synced_lyrics.is_some());
        assert!(resp.plain_lyrics.is_some());

        let json2 = r#"{"syncedLyrics":null,"plainLyrics":null}"#;
        let resp2: LrclibResponse = serde_json::from_str(json2).unwrap();
        assert!(resp2.synced_lyrics.is_none());
        assert!(resp2.plain_lyrics.is_none());
    }

    #[test]
    fn test_parse_plex_json_synced() {
        let json = r#"{
            "MediaContainer": {
                "Lyrics": [{
                    "Line": [
                        {"Span": [{"text": "Hello world", "startOffset": 12340}]},
                        {"Span": [{"text": "Second line", "startOffset": 25000}]}
                    ]
                }]
            }
        }"#;
        let lines = parse_plex_json_lyrics(json.as_bytes()).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello world");
        assert!((lines[0].timestamp.unwrap() - 12.34).abs() < 0.01);
        assert_eq!(lines[1].text, "Second line");
        assert!((lines[1].timestamp.unwrap() - 25.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_plex_json_unsynced() {
        let json = r#"{
            "MediaContainer": {
                "Lyrics": [{
                    "Line": [
                        {"Span": [{"text": "No timing here"}]},
                        {"Span": [{"text": "Just text"}]}
                    ]
                }]
            }
        }"#;
        let lines = parse_plex_json_lyrics(json.as_bytes()).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].timestamp.is_none());
        assert_eq!(lines[0].text, "No timing here");
    }

    #[test]
    fn test_parse_plex_json_multi_span() {
        let json = r#"{
            "MediaContainer": {
                "Lyrics": [{
                    "Line": [
                        {"Span": [
                            {"text": "Hello ", "startOffset": 5000},
                            {"text": "world"}
                        ]}
                    ]
                }]
            }
        }"#;
        let lines = parse_plex_json_lyrics(json.as_bytes()).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Hello world");
        assert!((lines[0].timestamp.unwrap() - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_plex_json_empty_lyrics() {
        let json = r#"{"MediaContainer": {"Lyrics": []}}"#;
        assert!(parse_plex_json_lyrics(json.as_bytes()).is_none());
    }

    #[test]
    fn test_parse_plex_json_invalid() {
        assert!(parse_plex_json_lyrics(b"not json").is_none());
        assert!(parse_plex_json_lyrics(b"{}").is_none());
    }

    #[test]
    fn test_parse_plex_json_skips_empty_text() {
        let json = r#"{
            "MediaContainer": {
                "Lyrics": [{
                    "Line": [
                        {"Span": [{"text": "  ", "startOffset": 1000}]},
                        {"Span": [{"text": "Real line", "startOffset": 2000}]}
                    ]
                }]
            }
        }"#;
        let lines = parse_plex_json_lyrics(json.as_bytes()).unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Real line");
    }

    #[test]
    fn test_validate_lyrics_path_valid() {
        assert!(validate_lyrics_path("/library/streams/123/lyrics"));
        assert!(validate_lyrics_path("/file/lyrics/song.lrc"));
    }

    #[test]
    fn test_validate_lyrics_path_rejects_traversal() {
        assert!(!validate_lyrics_path("/library/../etc/passwd"));
        assert!(!validate_lyrics_path("/library/%2e%2e/secret"));
    }

    #[test]
    fn test_validate_lyrics_path_rejects_wrong_prefix() {
        assert!(!validate_lyrics_path("/etc/passwd"));
        assert!(!validate_lyrics_path("/other/path"));
        assert!(!validate_lyrics_path("library/no-leading-slash"));
    }

    async fn lrclib_outcome(template: wiremock::ResponseTemplate) -> LyricsOutcome {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/get"))
            .respond_with(template)
            .mount(&mock_server)
            .await;
        let http = reqwest::Client::new();
        fetch_from_lrclib_at(&http, &mock_server.uri(), "Test", "Artist", "Album", 180.0).await
    }

    #[tokio::test]
    async fn test_fetch_from_lrclib_synced() {
        let outcome = lrclib_outcome(wiremock::ResponseTemplate::new(200).set_body_json(
            serde_json::json!({
                "syncedLyrics": "[00:05.00] Line one\n[00:10.00] Line two",
                "plainLyrics": "Line one\nLine two"
            }),
        ))
        .await;
        match outcome {
            LyricsOutcome::Found(result) => {
                assert!(result.is_synced);
                assert_eq!(result.lines.len(), 2);
                assert_eq!(result.source, LyricsSource::Lrclib);
            }
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_fetch_from_lrclib_plain_fallback() {
        let outcome = lrclib_outcome(wiremock::ResponseTemplate::new(200).set_body_json(
            serde_json::json!({
                "syncedLyrics": null,
                "plainLyrics": "Just plain text\nSecond line"
            }),
        ))
        .await;
        match outcome {
            LyricsOutcome::Found(result) => {
                assert!(!result.is_synced);
                assert_eq!(result.lines.len(), 2);
            }
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_fetch_from_lrclib_404_is_not_found() {
        let outcome = lrclib_outcome(wiremock::ResponseTemplate::new(404)).await;
        assert_eq!(outcome, LyricsOutcome::NotFound);
    }

    #[tokio::test]
    async fn test_fetch_from_lrclib_5xx_is_transient() {
        let outcome = lrclib_outcome(wiremock::ResponseTemplate::new(503)).await;
        assert_eq!(outcome, LyricsOutcome::Transient);
    }

    #[tokio::test]
    async fn test_fetch_from_lrclib_429_is_transient() {
        let outcome = lrclib_outcome(wiremock::ResponseTemplate::new(429)).await;
        assert_eq!(outcome, LyricsOutcome::Transient);
    }

    #[tokio::test]
    async fn test_fetch_from_lrclib_empty_body_is_not_found() {
        let outcome = lrclib_outcome(wiremock::ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "syncedLyrics": null, "plainLyrics": null }),
        ))
        .await;
        assert_eq!(outcome, LyricsOutcome::NotFound);
    }

    #[tokio::test]
    async fn test_fetch_from_lrclib_malformed_json_is_not_found() {
        let outcome =
            lrclib_outcome(wiremock::ResponseTemplate::new(200).set_body_string("not json")).await;
        assert_eq!(outcome, LyricsOutcome::NotFound);
    }

    #[tokio::test]
    async fn test_fetch_from_lrclib_connection_error_is_transient() {
        // Nothing is listening on this port → reqwest send error → Transient.
        let http = reqwest::Client::new();
        let outcome =
            fetch_from_lrclib_at(&http, "http://127.0.0.1:1", "T", "A", "Al", 180.0).await;
        assert_eq!(outcome, LyricsOutcome::Transient);
    }
}
