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

/// Fetch lyrics from LRCLIB. Tries synced (LRC) first, falls back to plain text.
pub async fn fetch_from_lrclib(
    http: &reqwest::Client,
    track_name: &str,
    artist_name: &str,
    album_name: &str,
    duration_secs: f64,
) -> Option<LyricsResult> {
    let duration_int = duration_secs as u64;
    let resp = http
        .get("https://lrclib.net/api/get")
        .query(&[
            ("track_name", track_name),
            ("artist_name", artist_name),
            ("album_name", album_name),
            ("duration", &duration_int.to_string()),
        ])
        .header(
            "Lrclib-Client",
            "ramus v0.9.1 (https://github.com/1337raspberry/ramus)",
        )
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let body = resp.bytes().await.ok()?;
    if body.len() > LRCLIB_MAX_RESPONSE {
        return None;
    }

    let parsed: LrclibResponse = serde_json::from_slice(&body).ok()?;

    if let Some(synced) = parsed.synced_lyrics {
        let lines = parse_lrc(&synced);
        if !lines.is_empty() {
            return Some(LyricsResult {
                is_synced: true,
                lines,
                source: LyricsSource::Lrclib,
            });
        }
    }

    if let Some(plain) = parsed.plain_lyrics {
        let lines = parse_plain_lyrics(&plain);
        if !lines.is_empty() {
            return Some(LyricsResult {
                is_synced: false,
                lines,
                source: LyricsSource::Lrclib,
            });
        }
    }

    None
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

    #[tokio::test]
    async fn test_fetch_from_lrclib_synced() {
        let mock_server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/get"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({
                    "syncedLyrics": "[00:05.00] Line one\n[00:10.00] Line two",
                    "plainLyrics": "Line one\nLine two"
                }),
            ))
            .mount(&mock_server)
            .await;

        let http = reqwest::Client::new();
        let resp = http
            .get(format!("{}/api/get", mock_server.uri()))
            .query(&[
                ("track_name", "Test"),
                ("artist_name", "Artist"),
                ("album_name", "Album"),
                ("duration", "180"),
            ])
            .header(
                "Lrclib-Client",
                "ramus v0.9.1 (https://github.com/1337raspberry/ramus)",
            )
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .unwrap();

        let body = resp.bytes().await.unwrap();
        let parsed: LrclibResponse = serde_json::from_slice(&body).unwrap();

        let lines = parse_lrc(parsed.synced_lyrics.as_deref().unwrap());
        assert_eq!(lines.len(), 2);
        assert!(lines[0].timestamp.is_some());
    }

    #[tokio::test]
    async fn test_fetch_from_lrclib_plain_fallback() {
        let mock_server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/get"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({
                    "syncedLyrics": null,
                    "plainLyrics": "Just plain text\nSecond line"
                }),
            ))
            .mount(&mock_server)
            .await;

        let http = reqwest::Client::new();
        let resp = http
            .get(format!("{}/api/get", mock_server.uri()))
            .send()
            .await
            .unwrap();

        let body = resp.bytes().await.unwrap();
        let parsed: LrclibResponse = serde_json::from_slice(&body).unwrap();

        assert!(parsed.synced_lyrics.is_none());
        let lines = parse_plain_lyrics(parsed.plain_lyrics.as_deref().unwrap());
        assert_eq!(lines.len(), 2);
        assert!(lines[0].timestamp.is_none());
    }

    #[tokio::test]
    async fn test_fetch_from_lrclib_not_found() {
        let mock_server = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/get"))
            .respond_with(wiremock::ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let http = reqwest::Client::new();
        let resp = http
            .get(format!("{}/api/get", mock_server.uri()))
            .send()
            .await
            .unwrap();

        assert!(!resp.status().is_success());
    }
}
