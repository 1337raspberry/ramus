//! Segmenting a genre description into renderable parts.
//!
//! A description is plain prose with two kinds of explicit markup:
//!   * `**Genre Name**` — a genre cross-reference.
//!   * `{{Artist Name}}` — an artist mention.
//!
//! Both are resolved into a single ordered segment list. Each link also carries
//! whether the referenced entity is present in the library (a genre with albums,
//! or an owned artist), which the UI uses to style and gate it: genre links
//! always drill into the genre's info, but artist links are only followable when
//! owned, and the library flag drives the accent/weight/underline treatment.

use std::collections::{HashMap, HashSet};

/// One rendered piece of a description.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DescriptionSegment {
    /// Plain prose (may contain newlines).
    Text { value: String },
    /// A genre cross-reference. `in_library` = the genre has albums in the library.
    GenreLink { value: String, in_library: bool },
    /// An artist mention. `value` is the description's display text; when owned,
    /// `nav_name` carries the library's actual artist name to navigate to (which
    /// may differ in punctuation/casing from the display text).
    ArtistLink {
        value: String,
        in_library: bool,
        nav_name: Option<String>,
    },
}

/// Normalise an artist name for tolerant matching between a description's
/// display text and the library's stored name: lowercase, keep only
/// alphanumerics. Folds away differences in spacing, hyphenation and other
/// punctuation so "blink-182" / "Blink 182" / "blink‐182" all collapse to
/// "blink182".
pub fn normalize_artist(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn push_text(out: &mut Vec<DescriptionSegment>, text: &str, start: usize, end: usize) {
    if end > start {
        out.push(DescriptionSegment::Text {
            value: text[start..end].to_string(),
        });
    }
}

/// Split a description into ordered segments, resolving `**genre**` and
/// `{{artist}}` markup. `library_genres` is a lowercased set of genres that have
/// albums. `library_artists` maps `normalize_artist`ed names to the library's
/// actual (display) artist name, so a tolerant match still navigates correctly.
pub fn build_description_segments(
    text: &str,
    library_genres: &HashSet<String>,
    library_artists: &HashMap<String, String>,
) -> Vec<DescriptionSegment> {
    let bytes = text.as_bytes();
    let mut out: Vec<DescriptionSegment> = Vec::new();
    let mut i = 0;
    let mut text_start = 0;
    // "No closer ahead" is monotone: once a forward `find` fails at some
    // position it fails at every later one. Memoise the failure so a long
    // run of unclosed openers scans forward once instead of per position —
    // without this, a degenerate imported description (megabytes of `{{`)
    // makes the loop quadratic while holding the mapper read lock.
    let mut no_genre_close = false;
    let mut no_artist_close = false;

    while i < bytes.len() {
        // **genre**
        if !no_genre_close && bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            match text[i + 2..].find("**") {
                Some(rel) => {
                    let inner = text[i + 2..i + 2 + rel].trim();
                    if !inner.is_empty() {
                        push_text(&mut out, text, text_start, i);
                        out.push(DescriptionSegment::GenreLink {
                            in_library: library_genres.contains(&inner.to_lowercase()),
                            value: inner.to_string(),
                        });
                        i += 2 + rel + 2;
                        text_start = i;
                        continue;
                    }
                }
                None => no_genre_close = true,
            }
        }
        // {{artist}}
        if !no_artist_close && bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            match text[i + 2..].find("}}") {
                Some(rel) => {
                    let inner = text[i + 2..i + 2 + rel].trim();
                    if !inner.is_empty() {
                        push_text(&mut out, text, text_start, i);
                        let key = normalize_artist(inner);
                        // All-punctuation names normalize to nothing; fall
                        // back to the exact-name slot the caller keys behind
                        // a NUL prefix, so "!!!" still links against an owned
                        // "!!!" without any cross-name looseness.
                        let nav = if key.is_empty() {
                            library_artists.get(&format!("\u{0}{}", inner.to_lowercase()))
                        } else {
                            library_artists.get(&key)
                        };
                        out.push(DescriptionSegment::ArtistLink {
                            value: inner.to_string(),
                            in_library: nav.is_some(),
                            nav_name: nav.cloned(),
                        });
                        i += 2 + rel + 2;
                        text_start = i;
                        continue;
                    }
                }
                None => no_artist_close = true,
            }
        }
        i += 1;
    }
    push_text(&mut out, text, text_start, bytes.len());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_lowercase()).collect()
    }

    /// Build a library-artist map keyed by `normalize_artist`, mirroring the
    /// command layer (including its NUL-prefixed exact keys for names that
    /// normalize to nothing).
    fn artists(names: &[&str]) -> HashMap<String, String> {
        names
            .iter()
            .map(|n| {
                let key = normalize_artist(n);
                if key.is_empty() {
                    (format!("\u{0}{}", n.trim().to_lowercase()), n.to_string())
                } else {
                    (key, n.to_string())
                }
            })
            .collect()
    }

    fn artist(value: &str, in_library: bool, nav: Option<&str>) -> DescriptionSegment {
        DescriptionSegment::ArtistLink {
            value: value.into(),
            in_library,
            nav_name: nav.map(|s| s.to_string()),
        }
    }

    #[test]
    fn punctuation_only_artists_match_exactly_not_loosely() {
        let lib = artists(&["!!!"]);
        // Exact all-punctuation reference resolves against the owned name…
        let segs = build_description_segments("see {{!!!}} live", &set(&[]), &lib);
        assert_eq!(segs[1], artist("!!!", true, Some("!!!")));
        // …but a different all-punctuation reference must NOT collide.
        let segs = build_description_segments("see {{...}} live", &set(&[]), &lib);
        assert_eq!(segs[1], artist("...", false, None));
    }

    #[test]
    fn unclosed_openers_stay_plain_text_in_linear_time() {
        // A long run of openers with no closer must come back as one text
        // segment — and quickly. Without the no-closer memo the loop
        // rescans to end-of-string from every position (quadratic), which
        // a degenerate imported description could weaponise.
        let hostile = "{{".repeat(100_000) + &"**".repeat(100_000);
        let segs = build_description_segments(&hostile, &set(&[]), &artists(&[]));
        assert_eq!(segs, vec![DescriptionSegment::Text { value: hostile }]);
    }

    #[test]
    fn markup_after_unclosed_opener_of_other_kind_still_parses() {
        // An unclosed `{{` must not stop later `**…**` from tokenizing
        // (and vice versa) — the memo flags are per-delimiter.
        let segs = build_description_segments(
            "broken {{ then **Punk Rock** after",
            &set(&["Punk Rock"]),
            &artists(&[]),
        );
        assert_eq!(
            segs,
            vec![
                DescriptionSegment::Text { value: "broken {{ then ".into() },
                DescriptionSegment::GenreLink { value: "Punk Rock".into(), in_library: true },
                DescriptionSegment::Text { value: " after".into() },
            ]
        );
    }

    #[test]
    fn parses_genre_and_artist_markup() {
        let segs = build_description_segments(
            "a **Punk Rock** act like {{Regina Spektor}} here",
            &set(&["Punk Rock"]),
            &artists(&["Regina Spektor"]),
        );
        assert_eq!(
            segs,
            vec![
                DescriptionSegment::Text { value: "a ".into() },
                DescriptionSegment::GenreLink { value: "Punk Rock".into(), in_library: true },
                DescriptionSegment::Text { value: " act like ".into() },
                artist("Regina Spektor", true, Some("Regina Spektor")),
                DescriptionSegment::Text { value: " here".into() },
            ]
        );
    }

    #[test]
    fn flags_out_of_library_references() {
        let segs = build_description_segments(
            "{{Lach}} founded it, see **Folk Punk**",
            &set(&[]),
            &artists(&[]),
        );
        assert_eq!(
            segs,
            vec![
                artist("Lach", false, None),
                DescriptionSegment::Text { value: " founded it, see ".into() },
                DescriptionSegment::GenreLink { value: "Folk Punk".into(), in_library: false },
            ]
        );
    }

    #[test]
    fn tolerant_artist_match_navigates_by_library_name() {
        // Description says "blink-182"; library stores "Blink 182". They should
        // match, and navigation must use the library's actual name.
        let segs = build_description_segments(
            "pioneered by {{blink-182}} in the 90s",
            &set(&[]),
            &artists(&["Blink 182"]),
        );
        assert_eq!(segs[1], artist("blink-182", true, Some("Blink 182")));
    }

    #[test]
    fn preserves_newlines_in_text() {
        let segs = build_description_segments("first line\n\nsecond line", &set(&[]), &artists(&[]));
        assert_eq!(
            segs,
            vec![DescriptionSegment::Text { value: "first line\n\nsecond line".into() }]
        );
    }

    #[test]
    fn membership_is_case_insensitive() {
        let segs = build_description_segments(
            "{{aphex twin}} and **JAZZ**",
            &set(&["Jazz"]),
            &artists(&["Aphex Twin"]),
        );
        assert_eq!(segs[0], artist("aphex twin", true, Some("Aphex Twin")));
        assert_eq!(segs[2], DescriptionSegment::GenreLink { value: "JAZZ".into(), in_library: true });
    }

    #[test]
    fn empty_markup_is_literal_text() {
        let segs = build_description_segments("an empty **** and {{}} here", &set(&[]), &artists(&[]));
        assert_eq!(
            segs,
            vec![DescriptionSegment::Text { value: "an empty **** and {{}} here".into() }]
        );
    }

    #[test]
    fn unclosed_markup_stays_text() {
        let segs =
            build_description_segments("a **dangling start", &set(&["dangling start"]), &artists(&[]));
        assert_eq!(
            segs,
            vec![DescriptionSegment::Text { value: "a **dangling start".into() }]
        );
    }

    #[test]
    fn plain_prose_is_single_segment() {
        let segs = build_description_segments("nothing to see", &set(&[]), &artists(&[]));
        assert_eq!(segs, vec![DescriptionSegment::Text { value: "nothing to see".into() }]);
    }

    #[test]
    fn normalize_artist_folds_punctuation_and_case() {
        assert_eq!(normalize_artist("blink-182"), "blink182");
        assert_eq!(normalize_artist("Blink 182"), "blink182");
        assert_eq!(normalize_artist("The Offspring"), "theoffspring");
        assert_eq!(normalize_artist("AC/DC"), "acdc");
    }
}
