use crate::models::{RangeField, RangeOp};

// --- Types ---

#[derive(Debug, Clone, PartialEq)]
pub enum SearchFilter {
    FreeText(String),
    Genre(String),
    Artist(String),
    AlbumTitle(String),
    TrackSearch(String),
    Range(RangeField, RangeOp, f64),
    Favourites,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedQuery {
    pub filters: Vec<SearchFilter>,
}

impl ParsedQuery {
    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    pub fn free_text(&self) -> Option<&str> {
        self.filters.iter().find_map(|f| match f {
            SearchFilter::FreeText(t) => Some(t.as_str()),
            _ => None,
        })
    }

    pub fn genre_filters(&self) -> Vec<&str> {
        self.filters
            .iter()
            .filter_map(|f| match f {
                SearchFilter::Genre(g) => Some(g.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn artist_filters(&self) -> Vec<&str> {
        self.filters
            .iter()
            .filter_map(|f| match f {
                SearchFilter::Artist(a) => Some(a.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn album_title_filters(&self) -> Vec<&str> {
        self.filters
            .iter()
            .filter_map(|f| match f {
                SearchFilter::AlbumTitle(a) => Some(a.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn track_searches(&self) -> Vec<&str> {
        self.filters
            .iter()
            .filter_map(|f| match f {
                SearchFilter::TrackSearch(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn range_filters(&self) -> Vec<(RangeField, RangeOp, f64)> {
        self.filters
            .iter()
            .filter_map(|f| match f {
                SearchFilter::Range(field, op, val) => Some((*field, *op, *val)),
                _ => None,
            })
            .collect()
    }

    pub fn has_track_search(&self) -> bool {
        self.filters
            .iter()
            .any(|f| matches!(f, SearchFilter::TrackSearch(_)))
    }

    pub fn has_favourites_filter(&self) -> bool {
        self.filters
            .iter()
            .any(|f| matches!(f, SearchFilter::Favourites))
    }

    pub fn is_free_text_only(&self) -> bool {
        self.filters.len() == 1 && self.free_text().is_some()
    }
}

// --- Parser ---

pub struct QueryParser;

impl QueryParser {
    /// Parse a raw query string into structured filters.
    /// Each operator consumes all text until an explicit `AND` delimiter.
    /// Without `AND`, the entire input belongs to the first operator.
    pub fn parse(input: &str) -> ParsedQuery {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return ParsedQuery {
                filters: Vec::new(),
            };
        }

        // Split on " AND " (case-sensitive, literal uppercase)
        let segments: Vec<&str> = trimmed.split(" AND ").collect();
        let mut filters = Vec::new();

        for segment in segments {
            let s = segment.trim();
            if s.is_empty() {
                continue;
            }
            if let Some(filter) = Self::parse_segment(s) {
                filters.push(filter);
            }
        }

        ParsedQuery { filters }
    }

    fn parse_segment(segment: &str) -> Option<SearchFilter> {
        let lower = segment.to_lowercase();

        if let Some(rest) = segment.strip_prefix('/') {
            let value = rest.trim();
            if value.is_empty() {
                return None;
            }
            Some(SearchFilter::Genre(value.to_string()))
        } else if let Some(rest) = segment.strip_prefix('@') {
            let value = rest.trim();
            if value.is_empty() {
                return None;
            }
            Some(SearchFilter::Artist(value.to_string()))
        } else if let Some(rest) = segment.strip_prefix('!') {
            let value = rest.trim();
            if value.is_empty() {
                return None;
            }
            Some(SearchFilter::TrackSearch(value.to_string()))
        } else if let Some(rest) = segment.strip_prefix('%') {
            let value = rest.trim();
            if value.is_empty() {
                return None;
            }
            Some(SearchFilter::AlbumTitle(value.to_string()))
        } else if lower.starts_with("fav:") || lower.starts_with("favourites:") {
            Some(SearchFilter::Favourites)
        } else if let Some(rest) = segment.strip_prefix('$') {
            Self::parse_range(RangeField::Year, rest)
                .or_else(|| Some(SearchFilter::FreeText(segment.to_string())))
        } else if lower.starts_with("year:") {
            let rest = &segment[5..];
            Self::parse_range(RangeField::Year, rest)
                .or_else(|| Some(SearchFilter::FreeText(segment.to_string())))
        } else if lower.starts_with("rating:") {
            let rest = &segment[7..];
            Self::parse_range(RangeField::Rating, rest)
                .or_else(|| Some(SearchFilter::FreeText(segment.to_string())))
        } else {
            Some(SearchFilter::FreeText(segment.to_string()))
        }
    }

    fn parse_range(field: RangeField, value: &str) -> Option<SearchFilter> {
        let (op, num_str) = if let Some(rest) = value.strip_prefix(">=") {
            (RangeOp::GreaterOrEqual, rest)
        } else if let Some(rest) = value.strip_prefix("<=") {
            (RangeOp::LessOrEqual, rest)
        } else if let Some(rest) = value.strip_prefix('>') {
            (RangeOp::GreaterThan, rest)
        } else if let Some(rest) = value.strip_prefix('<') {
            (RangeOp::LessThan, rest)
        } else {
            (RangeOp::Equal, value)
        };

        let number: f64 = num_str.parse().ok()?;
        Some(SearchFilter::Range(field, op, number))
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_free_text() {
        let q = QueryParser::parse("hello world");
        assert_eq!(q.filters.len(), 1);
        assert_eq!(q.free_text(), Some("hello world"));
    }

    #[test]
    fn test_parse_genre_filter() {
        let q = QueryParser::parse("/metal");
        assert_eq!(q.genre_filters(), vec!["metal"]);
    }

    #[test]
    fn test_parse_artist_filter() {
        let q = QueryParser::parse("@metallica");
        assert_eq!(q.artist_filters(), vec!["metallica"]);
    }

    #[test]
    fn test_parse_album_title_filter() {
        let q = QueryParser::parse("%reign");
        assert_eq!(q.album_title_filters(), vec!["reign"]);
    }

    #[test]
    fn test_parse_track_search() {
        let q = QueryParser::parse("!paranoid");
        assert_eq!(q.track_searches(), vec!["paranoid"]);
        assert!(q.has_track_search());
    }

    #[test]
    fn test_parse_multi_word_track_search() {
        let q = QueryParser::parse("!Baby Blue");
        assert_eq!(q.track_searches(), vec!["Baby Blue"]);
        assert_eq!(q.free_text(), None);
    }

    #[test]
    fn test_parse_multi_word_album_title_search() {
        let q = QueryParser::parse("%OK Computer");
        assert_eq!(q.album_title_filters(), vec!["OK Computer"]);
        assert_eq!(q.free_text(), None);
    }

    #[test]
    fn test_parse_year_greater_than() {
        let q = QueryParser::parse("year:>2000");
        let r = q.range_filters();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].0, RangeField::Year);
        assert_eq!(r[0].1, RangeOp::GreaterThan);
        assert_eq!(r[0].2, 2000.0);
    }

    #[test]
    fn test_parse_year_equal() {
        let q = QueryParser::parse("year:1997");
        let r = q.range_filters();
        assert_eq!(r[0].1, RangeOp::Equal);
        assert_eq!(r[0].2, 1997.0);
    }

    #[test]
    fn test_parse_rating_greater_or_equal() {
        let q = QueryParser::parse("rating:>=8");
        let r = q.range_filters();
        assert_eq!(r[0].0, RangeField::Rating);
        assert_eq!(r[0].1, RangeOp::GreaterOrEqual);
        assert_eq!(r[0].2, 8.0);
    }

    #[test]
    fn test_parse_less_than_or_equal() {
        let q = QueryParser::parse("year:<=1990");
        let r = q.range_filters();
        assert_eq!(r[0].1, RangeOp::LessOrEqual);
        assert_eq!(r[0].2, 1990.0);
    }

    #[test]
    fn test_parse_combined_with_and() {
        let q = QueryParser::parse("/metal AND @slayer AND year:>1985 AND !reign");
        assert_eq!(q.genre_filters(), vec!["metal"]);
        assert_eq!(q.artist_filters(), vec!["slayer"]);
        assert_eq!(q.range_filters()[0].2, 1985.0);
        assert_eq!(q.track_searches(), vec!["reign"]);
    }

    #[test]
    fn test_multi_word_genre_without_and() {
        let q = QueryParser::parse("/post rock");
        assert_eq!(q.genre_filters(), vec!["post rock"]);
        assert_eq!(q.free_text(), None);
    }

    #[test]
    fn test_multi_word_artist_without_and() {
        let q = QueryParser::parse("@blue öyster cult");
        assert_eq!(q.artist_filters(), vec!["blue öyster cult"]);
        assert_eq!(q.free_text(), None);
    }

    #[test]
    fn test_operator_without_and_consumes_all() {
        let q = QueryParser::parse("/rock year:2000");
        assert_eq!(q.genre_filters(), vec!["rock year:2000"]);
        assert!(q.range_filters().is_empty());
    }

    #[test]
    fn test_parse_empty_input() {
        let q = QueryParser::parse("");
        assert!(q.is_empty());
    }

    #[test]
    fn test_parse_invalid_year() {
        let q = QueryParser::parse("year:abc");
        assert!(q.range_filters().is_empty());
        assert_eq!(q.free_text(), Some("year:abc"));
    }

    #[test]
    fn test_parse_bare_operators() {
        assert!(QueryParser::parse("/").is_empty());
        assert!(QueryParser::parse("@").is_empty());
        assert!(QueryParser::parse("!").is_empty());
        assert!(QueryParser::parse("%").is_empty());
    }


    #[test]
    fn test_default_search_does_not_produce_track_search() {
        let q = QueryParser::parse("radiohead");
        assert!(!q.has_track_search());
        assert_eq!(q.free_text(), Some("radiohead"));
    }

    #[test]
    fn test_is_free_text_only() {
        assert!(QueryParser::parse("radiohead").is_free_text_only());
        assert!(!QueryParser::parse("@radiohead").is_free_text_only());
        assert!(!QueryParser::parse("!paranoid").is_free_text_only());
        assert!(!QueryParser::parse("/rock AND radiohead").is_free_text_only());
        assert!(!QueryParser::parse("").is_free_text_only());
    }
}
