/// Lossless audio codecs (case-insensitive matching via `is_lossless_codec`).
pub const LOSSLESS_CODECS: &[&str] = &["flac", "alac", "wav", "aiff", "aif", "pcm"];

/// Returns `true` if `codec` is a lossless audio format (case-insensitive).
pub fn is_lossless_codec(codec: &str) -> bool {
    LOSSLESS_CODECS.contains(&codec.to_lowercase().as_str())
}

/// Escape a string for FTS5 MATCH queries: strip `"*():^{}+`, replace `-` with space.
pub fn escape_fts5(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '"' | '*' | '(' | ')' | ':' | '^' | '{' | '}' | '+' => {}
            '-' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape `%`, `_`, `\` for SQL LIKE patterns.
pub fn escape_like(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '%' | '_' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// RFC 3986 percent-encode a string (unreserved chars pass through).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    out
}

/// Percent-decode a string (e.g. `%2F` → `/`).
pub fn percent_decode(s: &str) -> String {
    let mut result = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                result.push(byte);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&result).into_owned()
}

/// Whether a URI's host resolves to the loopback interface. Used at the
/// plex.tv resources boundary to drop stale loopback connections that can
/// only be reached from the advertising device, never from us.
///
/// Matches IPv4 `127.x.x.x`, IPv6 `::1` (in bracket syntax), and the
/// `localhost` hostname. Malformed URIs return `false` (the downstream
/// probe will fail them anyway).
pub fn is_loopback_uri(uri: &str) -> bool {
    let Ok(url) = url::Url::parse(uri) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" {
        return true;
    }
    if let Some(stripped) = lower.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        // Bracketed IPv6 literal.
        if let Ok(addr) = stripped.parse::<std::net::Ipv6Addr>() {
            return addr.is_loopback();
        }
    }
    if let Ok(addr) = lower.parse::<std::net::Ipv4Addr>() {
        return addr.is_loopback();
    }
    if let Ok(addr) = lower.parse::<std::net::Ipv6Addr>() {
        return addr.is_loopback();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_loopback_uri_ipv4() {
        assert!(is_loopback_uri("http://127.0.0.1:56393"));
        assert!(is_loopback_uri("https://127.0.0.1:32400"));
        assert!(is_loopback_uri("http://127.7.7.7/"));
        assert!(!is_loopback_uri("https://192-168-0-1.xxx.plex.direct:32400"));
        assert!(!is_loopback_uri("https://10.0.0.5:32400"));
        assert!(!is_loopback_uri("http://192.168.1.100/"));
    }

    #[test]
    fn test_is_loopback_uri_localhost() {
        assert!(is_loopback_uri("http://localhost:8080"));
        assert!(is_loopback_uri("https://LOCALHOST/"));
    }

    #[test]
    fn test_is_loopback_uri_ipv6() {
        assert!(is_loopback_uri("http://[::1]:8080"));
        assert!(!is_loopback_uri("http://[2001:db8::1]:8080"));
    }

    #[test]
    fn test_is_loopback_uri_malformed() {
        assert!(!is_loopback_uri("not a url"));
        assert!(!is_loopback_uri(""));
    }

    #[test]
    fn test_lossless_codec_detection() {
        assert!(is_lossless_codec("flac"));
        assert!(is_lossless_codec("alac"));
        assert!(is_lossless_codec("wav"));
        assert!(is_lossless_codec("aiff"));
        assert!(is_lossless_codec("aif"));
        assert!(is_lossless_codec("pcm"));
        assert!(is_lossless_codec("FLAC"));
        assert!(!is_lossless_codec("mp3"));
        assert!(!is_lossless_codec("aac"));
        assert!(!is_lossless_codec("opus"));
        assert!(!is_lossless_codec("vorbis"));
    }

    #[test]
    fn test_fts5_escaping() {
        assert_eq!(escape_fts5("hello-world"), "hello world");
        assert_eq!(escape_fts5(r#"test"quote"#), "testquote");
        assert_eq!(escape_fts5("foo*bar"), "foobar");
        assert_eq!(escape_fts5("(group)"), "group");
        assert_eq!(escape_fts5("normal text"), "normal text");
        assert_eq!(escape_fts5("colon:value"), "colonvalue");
    }

    #[test]
    fn test_escape_fts5_hyphen_replaced_with_space() {
        assert_eq!(escape_fts5("-something"), " something");
        assert_eq!(escape_fts5("hip-hop"), "hip hop");
    }

    #[test]
    fn test_escape_fts5_strips_metacharacters() {
        let escaped = escape_fts5("hello*world\"test");
        assert_eq!(escaped, "helloworldtest");
    }

    #[test]
    fn test_escape_fts5_preserves_keywords() {
        let escaped = escape_fts5("rock OR metal");
        assert_eq!(escaped, "rock OR metal");
    }

    #[test]
    fn test_like_pattern_escaping() {
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("track_1"), "track\\_1");
        assert_eq!(escape_like("back\\slash"), "back\\\\slash");
        assert_eq!(escape_like("hello world"), "hello world");
        assert_eq!(escape_like(""), "");
        assert_eq!(escape_like("%_\\"), "\\%\\_\\\\");
        assert_eq!(escape_like("foo%bar_baz"), "foo\\%bar\\_baz");
        assert_eq!(escape_like("%%"), "\\%\\%");
        assert_eq!(escape_like("björk"), "björk");
    }

    #[test]
    fn test_percent_encode() {
        assert_eq!(percent_encode("abc123"), "abc123");
        assert_eq!(percent_encode("hello world"), "hello%20world");
        assert_eq!(percent_encode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn test_percent_decode() {
        assert_eq!(percent_decode("abc123"), "abc123");
        assert_eq!(percent_decode("%2e%2e"), "..");
        assert_eq!(percent_decode("/library/%2e%2e/etc"), "/library/../etc");
    }
}
