//! Smart-playlist filter grammar.
//!
//! A smart playlist stores no track list — it stores a library search query
//! that the server re-runs on every read. This module owns the constrained
//! shape the app can build (and edit): a flat AND of terms, where each term
//! matches one field with one operator against one or more values. Values
//! within a term are OR alternatives, comma-joined on the wire. Plex's full
//! grammar additionally allows nested and/or groups (`push`/`pop` markers);
//! those don't fit the shape and parse to `None`, which callers surface as
//! "view only" rather than guessing at semantics.

use serde::{Deserialize, Serialize};

use crate::util::{percent_decode, percent_encode};

/// Metadata type code for tracks — smart audio playlists query at track
/// level (album/artist fields still apply via dotted scopes).
const TRACK_TYPE: &str = "10";

/// One filter operator. On the wire the operator is a suffix on the field
/// name, completed by the query pair's own `=` (`year>>=1990` is "year
/// greater than 1990"). The exact-match string flavours instead lead the
/// value with an extra `=` (`title==x` splits into key `title`, value `=x`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SmartOp {
    /// Tag/integer "is"; string "contains"; boolean "is true".
    Eq,
    /// Tag/integer "is not"; string "does not contain"; boolean "is false".
    NotEq,
    /// String exact match.
    Exact,
    /// String exact mismatch.
    NotExact,
    /// Integer strictly greater; date "after". An inclusive "N or more"
    /// therefore sends N-1. Relative date values (`-30d`) mean "within the
    /// last 30 days".
    Gt,
    /// Integer strictly less; date "before". Items with no value for the
    /// field match too (a never-played track counts as "played before any
    /// date").
    Lt,
    /// String begins-with.
    BeginsWith,
    /// String ends-with.
    EndsWith,
}

impl SmartOp {
    /// Suffix appended to the field name on the wire; the pair's `=`
    /// completes every operator key.
    fn wire_suffix(self) -> &'static str {
        match self {
            SmartOp::Eq => "",
            SmartOp::NotEq => "!",
            SmartOp::Exact => "=",
            SmartOp::NotExact => "!=",
            SmartOp::Gt => ">>",
            SmartOp::Lt => "<<",
            SmartOp::BeginsWith => "<",
            SmartOp::EndsWith => ">",
        }
    }
}

/// One rule: `field op (v1 OR v2 OR …)`. `field` is a fully scoped key
/// (`album.genre`, `track.userRating`) — bare names are legal Plex but
/// ambiguous (a bare `year` on a track query silently matches nothing), so
/// builders should always scope explicitly. Tag fields take tag ids as
/// values, never names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartTerm {
    pub field: String,
    pub op: SmartOp,
    pub values: Vec<String>,
}

/// The constrained filter: terms AND together, `sort` and `limit` ride the
/// same query. `sort` keeps the raw wire value (`random`, `userRating:desc`,
/// or a comma-joined multi-key) so sorts we don't model still round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SmartFilter {
    #[serde(default)]
    pub terms: Vec<SmartTerm>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Build the `uri` the playlist create/update endpoints take for a smart
/// playlist — the same server-scoped envelope as
/// [`super::client::build_library_uri`], pointing at a section search
/// instead of a track list. Values are percent-encoded individually (the
/// server splits pairs and commas before decoding elements, so encoding is
/// what protects a literal `,`/`&` inside a value); the whole uri is later
/// transport-encoded once more as a query-param value.
pub fn build_smart_uri(
    machine_identifier: &str,
    section_key: &str,
    filter: &SmartFilter,
) -> String {
    let mut query = format!("type={}", TRACK_TYPE);
    for term in &filter.terms {
        if term.field.is_empty() || term.values.is_empty() {
            continue;
        }
        let values: Vec<String> = term.values.iter().map(|v| percent_encode(v)).collect();
        query.push_str(&format!(
            "&{}{}={}",
            term.field,
            term.op.wire_suffix(),
            values.join(",")
        ));
    }
    if let Some(sort) = filter.sort.as_deref() {
        if !sort.is_empty() {
            query.push_str(&format!("&sort={}", sort));
        }
    }
    if let Some(limit) = filter.limit {
        query.push_str(&format!("&limit={}", limit));
    }
    format!(
        "server://{}/com.plexapp.plugins.library/library/sections/{}/all?{}",
        machine_identifier, section_key, query
    )
}

/// Parse a smart playlist's stored `content` uri back into the constrained
/// shape. Returns `None` for anything that doesn't fit — nested and/or
/// groups, non-track queries, malformed limits.
///
/// Accepts both forms seen in the wild: the server's normalized
/// `library://x/directory/<percent-encoded section query>` and the
/// `server://…/library/sections/<key>/all?<query>` shape sent at create
/// time. Pairs are split *before* decoding and each side decoded afterwards
/// — values with reserved characters are double-encoded at rest, and
/// decoding first would conjure phantom separators.
pub fn parse_smart_content(content: &str) -> Option<SmartFilter> {
    // Normalize to a plain `/library/sections/…/all?query` string.
    let decoded = if let Some(idx) = content.find("/directory/") {
        percent_decode(&content[idx + "/directory/".len()..])
    } else if let Some(idx) = content.find("://") {
        let after = &content[idx + 3..];
        after[after.find('/')?..].to_string()
    } else {
        content.to_string()
    };

    let (path, query) = decoded.split_once('?')?;
    if !path.trim_end_matches('/').ends_with("/all") {
        return None;
    }

    let mut filter = SmartFilter::default();
    let mut saw_track_type = false;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (raw_key, raw_value) = pair.split_once('=')?;
        let key = percent_decode(raw_key);

        // Group markers mean nested and/or logic — outside the shape.
        if key == "push" || key == "pop" || key == "or" || key == "and" {
            return None;
        }
        match key.as_str() {
            "type" => {
                if raw_value != TRACK_TYPE {
                    return None;
                }
                saw_track_type = true;
                continue;
            }
            "sort" => {
                filter.sort = Some(percent_decode(raw_value));
                continue;
            }
            "limit" => {
                filter.limit = Some(percent_decode(raw_value).parse().ok()?);
                continue;
            }
            _ => {}
        }

        let (field, base_op) = if let Some(f) = key.strip_suffix(">>") {
            (f, SmartOp::Gt)
        } else if let Some(f) = key.strip_suffix("<<") {
            (f, SmartOp::Lt)
        } else if let Some(f) = key.strip_suffix('!') {
            (f, SmartOp::NotEq)
        } else if let Some(f) = key.strip_suffix('<') {
            (f, SmartOp::BeginsWith)
        } else if let Some(f) = key.strip_suffix('>') {
            (f, SmartOp::EndsWith)
        } else {
            (key.as_str(), SmartOp::Eq)
        };
        if field.is_empty() {
            return None;
        }

        // A leading `=` in the value upgrades to the exact-match flavour.
        let (op, value_part) = match (base_op, raw_value.strip_prefix('=')) {
            (SmartOp::Eq, Some(rest)) => (SmartOp::Exact, rest),
            (SmartOp::NotEq, Some(rest)) => (SmartOp::NotExact, rest),
            _ => (base_op, raw_value),
        };

        filter.terms.push(SmartTerm {
            field: field.to_string(),
            op,
            values: value_part.split(',').map(percent_decode).collect(),
        });
    }

    if !saw_track_type {
        return None;
    }
    Some(filter)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(field: &str, op: SmartOp, values: &[&str]) -> SmartTerm {
        SmartTerm {
            field: field.into(),
            op,
            values: values.iter().map(|v| v.to_string()).collect(),
        }
    }

    #[test]
    fn test_build_smart_uri_shape() {
        let filter = SmartFilter {
            terms: vec![
                term("album.genre", SmartOp::Eq, &["419916", "464672"]),
                term("artist.title", SmartOp::Eq, &["daft"]),
                term("album.year", SmartOp::Gt, &["1999"]),
            ],
            sort: Some("random".into()),
            limit: Some(50),
        };
        assert_eq!(
            build_smart_uri("mid", "12", &filter),
            "server://mid/com.plexapp.plugins.library/library/sections/12/all\
             ?type=10&album.genre=419916,464672&artist.title=daft\
             &album.year>>=1999&sort=random&limit=50"
        );
    }

    #[test]
    fn test_round_trip_preserves_filter() {
        let filter = SmartFilter {
            terms: vec![
                term("album.genre", SmartOp::Eq, &["1", "2"]),
                term("track.userRating", SmartOp::Gt, &["7"]),
                term("track.lastViewedAt", SmartOp::Lt, &["-90d"]),
                term("artist.title", SmartOp::NotEq, &["live"]),
                term("track.title", SmartOp::Exact, &["Intro"]),
                term("album.title", SmartOp::NotExact, &["Demos"]),
                term("track.title", SmartOp::BeginsWith, &["The"]),
                term("track.title", SmartOp::EndsWith, &["remix"]),
            ],
            sort: Some("userRating:desc".into()),
            limit: Some(100),
        };
        let uri = build_smart_uri("mid", "12", &filter);
        assert_eq!(parse_smart_content(&uri), Some(filter));
    }

    #[test]
    fn test_values_with_reserved_characters_round_trip() {
        // A comma inside a value must not split into two OR alternatives,
        // and `&`/`=` must not break pair parsing.
        let filter = SmartFilter {
            terms: vec![term(
                "artist.title",
                SmartOp::Eq,
                &["Crosby, Stills & Nash", "a=b"],
            )],
            sort: None,
            limit: None,
        };
        let uri = build_smart_uri("mid", "12", &filter);
        assert_eq!(parse_smart_content(&uri), Some(filter));
    }

    #[test]
    fn test_parse_server_normalized_directory_form() {
        // The form PMS stores after creation: `library://x/directory/` plus
        // the percent-encoded section query.
        let content = "library://x/directory/%2Flibrary%2Fsections%2F12%2Fall\
                       %3Ftype%3D10%26sort%3DtitleSort%26track%2EuserRating%3D10";
        assert_eq!(
            parse_smart_content(content),
            Some(SmartFilter {
                terms: vec![term("track.userRating", SmartOp::Eq, &["10"])],
                sort: Some("titleSort".into()),
                limit: None,
            })
        );
    }

    #[test]
    fn test_parse_rejects_group_markers() {
        let content = "/library/sections/12/all?type=10\
                       &push=1&album.genre=1&or=1&album.genre=2&pop=1";
        assert_eq!(parse_smart_content(content), None);
    }

    #[test]
    fn test_parse_rejects_non_track_queries() {
        assert_eq!(
            parse_smart_content("/library/sections/12/all?type=9&album.genre=1"),
            None
        );
        // No type at all is just as unanswerable.
        assert_eq!(
            parse_smart_content("/library/sections/12/all?album.genre=1"),
            None
        );
    }

    #[test]
    fn test_parse_rejects_malformed_limit_and_non_search_paths() {
        assert_eq!(
            parse_smart_content("/library/sections/12/all?type=10&limit=lots"),
            None
        );
        assert_eq!(
            parse_smart_content("/library/sections/12/genre?type=10"),
            None
        );
    }

    #[test]
    fn test_exact_match_marker_stays_in_value_for_other_ops() {
        // `year>>==5` is nonsense grammar; the `=` belongs to the value
        // rather than upgrading a comparison to "exact".
        let parsed = parse_smart_content("/library/sections/12/all?type=10&album.year>>==5");
        assert_eq!(
            parsed,
            Some(SmartFilter {
                terms: vec![term("album.year", SmartOp::Gt, &["=5"])],
                sort: None,
                limit: None,
            })
        );
    }

    #[test]
    fn test_zero_rule_filter_builds_limit_and_sort_only() {
        // "50 random tracks from the whole library" is a legitimate filter.
        let filter = SmartFilter {
            terms: vec![],
            sort: Some("random".into()),
            limit: Some(50),
        };
        let uri = build_smart_uri("mid", "12", &filter);
        assert_eq!(
            uri,
            "server://mid/com.plexapp.plugins.library/library/sections/12/all\
             ?type=10&sort=random&limit=50"
        );
        assert_eq!(parse_smart_content(&uri), Some(filter));
    }
}
