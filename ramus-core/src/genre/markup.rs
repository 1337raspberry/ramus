//! Segmenting a genre description into renderable parts.
//!
//! A description is plain prose with two kinds of embedded references:
//!   * `**Genre Name**` — explicit markup, always linked to that genre.
//!   * Bare artist names — unmarked, linked only when they match an artist the
//!     user actually has in their library (so a tap always lands somewhere).
//!
//! Both are resolved here into a single ordered, non-overlapping segment list.
//! Genre spans are taken from the markup first; artist names are then matched
//! only in the plain-text gaps between them, so the two never overlap.

use aho_corasick::{AhoCorasick, MatchKind};

use crate::genre::mapper::GenreMapper;

/// A single token's worth of license to also link bare single-word artist
/// names: only when at least this many characters, to keep common short band
/// names (e.g. "War", "Yes") from matching ordinary prose. Multi-word names
/// are always eligible.
const ARTIST_MIN_SINGLE_TOKEN_CHARS: usize = 6;

/// One rendered piece of a description.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum DescriptionSegment {
    /// Plain prose.
    Text(String),
    /// A genre cross-reference (from `**...**`); links to that genre.
    GenreLink(String),
    /// A library artist mentioned in the prose; links to that artist.
    ArtistLink(String),
}

/// A prebuilt scanner over the library's artist names. Only names eligible for
/// matching (multi-word, or single words long enough) become patterns, so an
/// ineligible name can never produce a false link.
pub struct ArtistIndex {
    automaton: Option<AhoCorasick>,
    /// Canonical (original-cased) display name per pattern id.
    canonical: Vec<String>,
}

impl ArtistIndex {
    /// Build from raw library artist names. Names are matched case-insensitively
    /// but displayed/navigated with their canonical casing.
    pub fn build(names: &[String]) -> Self {
        let mut patterns: Vec<String> = Vec::new();
        let mut canonical: Vec<String> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        for name in names {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            let lower = trimmed.to_lowercase();
            if !seen.insert(lower) {
                continue; // dedupe by lowercased name; first casing wins
            }
            let multi_word = trimmed.split_whitespace().nth(1).is_some();
            let long_enough = trimmed.chars().count() >= ARTIST_MIN_SINGLE_TOKEN_CHARS;
            if !(multi_word || long_enough) {
                continue;
            }
            patterns.push(trimmed.to_string());
            canonical.push(trimmed.to_string());
        }

        let automaton = if patterns.is_empty() {
            None
        } else {
            AhoCorasick::builder()
                .match_kind(MatchKind::LeftmostLongest)
                .ascii_case_insensitive(true)
                .build(&patterns)
                .ok()
        };

        Self {
            automaton,
            canonical,
        }
    }

    fn is_empty(&self) -> bool {
        self.automaton.is_none()
    }
}

/// Split a description into ordered, non-overlapping segments. Genre markup
/// (`**...**`) is resolved first; artist names are matched only in the prose
/// between genre spans. A name that is also a genre is left as text (the genre
/// markup is the authoritative way to link a genre).
pub fn build_description_segments(
    text: &str,
    mapper: &GenreMapper,
    artists: &ArtistIndex,
) -> Vec<DescriptionSegment> {
    let mut out: Vec<DescriptionSegment> = Vec::new();
    let mut last = 0;
    for (start, end, name) in genre_spans(text) {
        if start > last {
            scan_gap(&text[last..start], mapper, artists, &mut out);
        }
        if name.is_empty() {
            // Degenerate `****` — keep the literal asterisks as prose.
            out.push(DescriptionSegment::Text(text[start..end].to_string()));
        } else {
            out.push(DescriptionSegment::GenreLink(name));
        }
        last = end;
    }
    if last < text.len() {
        scan_gap(&text[last..], mapper, artists, &mut out);
    }
    out
}

/// Yield `(start, end, inner_name)` byte ranges for each `**...**` span. The
/// range covers the full `**...**`; `inner_name` is the trimmed contents.
fn genre_spans(text: &str) -> Vec<(usize, usize, String)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'*' && bytes[i + 1] == b'*' {
            if let Some(rel) = text[i + 2..].find("**") {
                let inner_start = i + 2;
                let inner_end = inner_start + rel;
                let end = inner_end + 2;
                spans.push((i, end, text[inner_start..inner_end].trim().to_string()));
                i = end;
                continue;
            }
            break; // unmatched opener — leave the rest as prose
        }
        i += 1;
    }
    spans
}

/// Append segments for a span of plain prose, linking any eligible artist name.
fn scan_gap(
    gap: &str,
    mapper: &GenreMapper,
    artists: &ArtistIndex,
    out: &mut Vec<DescriptionSegment>,
) {
    if gap.is_empty() {
        return;
    }
    let Some(automaton) = artists.automaton.as_ref().filter(|_| !artists.is_empty()) else {
        out.push(DescriptionSegment::Text(gap.to_string()));
        return;
    };

    let mut cursor = 0;
    for m in automaton.find_iter(gap) {
        let (s, e) = (m.start(), m.end());
        // Require non-alphanumeric boundaries so "War" can't match inside
        // "Warsaw" and a possessive "Spektor's" still links cleanly.
        let prev_ok = gap[..s].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let next_ok = gap[e..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if !prev_ok || !next_ok {
            continue;
        }
        let canonical = &artists.canonical[m.pattern().as_usize()];
        if mapper.is_known_genre_name(canonical) {
            continue; // a genre by this name links via markup, not as an artist
        }
        if s > cursor {
            out.push(DescriptionSegment::Text(gap[cursor..s].to_string()));
        }
        out.push(DescriptionSegment::ArtistLink(canonical.clone()));
        cursor = e;
    }
    if cursor < gap.len() {
        out.push(DescriptionSegment::Text(gap[cursor..].to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapper_with(genres: &[&str]) -> GenreMapper {
        let nodes: Vec<String> = genres
            .iter()
            .map(|g| format!(r#"{{"name":"{g}","children":[]}}"#))
            .collect();
        let json = format!(r#"{{"genres":[{}]}}"#, nodes.join(","));
        GenreMapper::from_json_bytes(json.as_bytes()).unwrap()
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn links_multiword_artist_in_prose() {
        let mapper = mapper_with(&["Anti-Folk"]);
        let artists = ArtistIndex::build(&names(&["Regina Spektor"]));
        let segs = build_description_segments(
            "later acts such as Regina Spektor.",
            &mapper,
            &artists,
        );
        assert!(segs.contains(&DescriptionSegment::ArtistLink("Regina Spektor".into())));
    }

    #[test]
    fn keeps_genre_markup_as_genre_link() {
        let mapper = mapper_with(&["Punk Rock"]);
        let artists = ArtistIndex::build(&names(&[]));
        let segs = build_description_segments("rooted in **Punk Rock** music", &mapper, &artists);
        assert_eq!(segs[0], DescriptionSegment::Text("rooted in ".into()));
        assert_eq!(segs[1], DescriptionSegment::GenreLink("Punk Rock".into()));
        assert_eq!(segs[2], DescriptionSegment::Text(" music".into()));
    }

    #[test]
    fn short_single_word_name_is_not_linked() {
        let mapper = mapper_with(&[]);
        let artists = ArtistIndex::build(&names(&["War", "Yes"]));
        let segs = build_description_segments("the war was over and yes it ended", &mapper, &artists);
        assert!(segs.iter().all(|s| matches!(s, DescriptionSegment::Text(_))));
    }

    #[test]
    fn long_single_word_name_is_linked() {
        let mapper = mapper_with(&[]);
        let artists = ArtistIndex::build(&names(&["Portishead"]));
        let segs = build_description_segments("pioneered by Portishead in Bristol", &mapper, &artists);
        assert!(segs.contains(&DescriptionSegment::ArtistLink("Portishead".into())));
    }

    #[test]
    fn word_boundary_prevents_substring_match() {
        let mapper = mapper_with(&[]);
        // "Air" is 3 chars so ineligible anyway; use a 6-char name embedded in a word.
        let artists = ArtistIndex::build(&names(&["Garden"]));
        let segs = build_description_segments("walking through the gardens", &mapper, &artists);
        assert!(segs.iter().all(|s| matches!(s, DescriptionSegment::Text(_))));
    }

    #[test]
    fn name_that_is_also_a_genre_is_not_artist_linked() {
        let mapper = mapper_with(&["Industrial"]);
        let artists = ArtistIndex::build(&names(&["Industrial"]));
        let segs = build_description_segments("a wave of industrial sound", &mapper, &artists);
        assert!(segs.iter().all(|s| matches!(s, DescriptionSegment::Text(_))));
    }

    #[test]
    fn artist_not_matched_inside_genre_span() {
        // An artist name that happens to sit inside a **genre** token must not
        // be artist-linked — genre spans are never scanned for artists.
        let mapper = mapper_with(&["Country Blues"]);
        let artists = ArtistIndex::build(&names(&["Country Blues"]));
        let segs = build_description_segments("see **Country Blues** here", &mapper, &artists);
        assert_eq!(segs[1], DescriptionSegment::GenreLink("Country Blues".into()));
        assert!(!segs
            .iter()
            .any(|s| matches!(s, DescriptionSegment::ArtistLink(_))));
    }

    #[test]
    fn possessive_after_name_still_links() {
        let mapper = mapper_with(&[]);
        let artists = ArtistIndex::build(&names(&["Regina Spektor"]));
        let segs = build_description_segments("Regina Spektor's debut", &mapper, &artists);
        assert_eq!(segs[0], DescriptionSegment::ArtistLink("Regina Spektor".into()));
    }

    #[test]
    fn empty_artist_index_returns_plain_text() {
        let mapper = mapper_with(&["Rock"]);
        let artists = ArtistIndex::build(&names(&[]));
        let segs = build_description_segments("just some prose", &mapper, &artists);
        assert_eq!(segs, vec![DescriptionSegment::Text("just some prose".into())]);
    }

    #[test]
    fn case_insensitive_match_keeps_canonical_casing() {
        let mapper = mapper_with(&[]);
        let artists = ArtistIndex::build(&names(&["Aphex Twin"]));
        let segs = build_description_segments("influenced by aphex twin heavily", &mapper, &artists);
        assert!(segs.contains(&DescriptionSegment::ArtistLink("Aphex Twin".into())));
    }
}
