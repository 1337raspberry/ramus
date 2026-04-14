use std::collections::HashSet;
use std::sync::Arc;

use crate::cache::db::CacheDatabase;
use crate::models::{RangeField, SearchResult, SearchResultKind};
use crate::search::parser::ParsedQuery;

// --- Genre expansion trait (decouples from GenreMapper) ---

/// Trait for expanding a genre name into all descendant genre names.
/// Implemented by GenreMapper in the genre module.
pub trait GenreExpander: Send + Sync {
    fn expand_genre(&self, name: &str) -> Option<HashSet<String>>;
}

// --- SearchEngine ---

pub struct SearchEngine {
    db: Arc<CacheDatabase>,
    genre_expander: Option<Arc<dyn GenreExpander>>,
}

impl SearchEngine {
    pub fn new(db: Arc<CacheDatabase>, genre_expander: Option<Arc<dyn GenreExpander>>) -> Self {
        Self {
            db,
            genre_expander,
        }
    }

    /// Execute a parsed query. Returns albums first, then tracks.
    pub fn search(&self, query: &ParsedQuery, limit: usize) -> Result<Vec<SearchResult>, crate::cache::db::CacheError> {
        if query.is_empty() {
            return Ok(Vec::new());
        }

        let album_ids = self.resolve_album_constraints(query)?;
        if let Some(ref ids) = album_ids {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
        }

        let has_album_searches = query.free_text().is_some()
            || !query.artist_filters().is_empty()
            || !query.album_title_filters().is_empty()
            || !query.genre_filters().is_empty()
            || !query.range_filters().is_empty()
            || query.has_favourites_filter();

        let mut results = Vec::new();

        if has_album_searches {
            let album_limit = if query.is_free_text_only() { 5 } else { limit };
            let album_results = self.search_albums(query, album_ids.as_ref(), album_limit)?;
            results.extend(album_results);
        }

        // Explicit ! operator track search
        if query.has_track_search() {
            let track_results = self.search_tracks(query, album_ids.as_ref(), limit)?;
            results.extend(track_results);
        }

        // Supplementary track search for free text (no ! operator)
        if let Some(text) = query.free_text() {
            if !query.has_track_search() {
                let track_results = self.search_tracks_by_text(text, album_ids.as_ref(), 10)?;
                results.extend(track_results);
            }
        }

        Ok(results)
    }

    // --- Album Search ---

    fn search_albums(
        &self,
        query: &ParsedQuery,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<SearchResult>, crate::cache::db::CacheError> {
        let mut seen = HashSet::new();
        let mut results = Vec::new();

        // % operator: search by album title
        for title_query in query.album_title_filters() {
            let rows = self.db.search_albums_by_title_filtered(title_query, album_ids, limit)?;
            for row in rows {
                if seen.insert(row.album_source_id.clone()) {
                    let score = match_score(&row.album_title, title_query);
                    results.push(SearchResult {
                        id: format!("album-{}", row.album_source_id),
                        kind: SearchResultKind::Album,
                        album_source_id: row.album_source_id,
                        album_title: row.album_title,
                        artist_name: row.artist_name,
                        year: row.year,
                        album_art_path: row.art_url,
                        track_source_id: None,
                        track_title: None,
                        track_artist: None,
                        is_favourite: row.is_favourite,
                        score,
                    });
                }
            }
        }

        // @ operator: search by artist name
        for artist_query in query.artist_filters() {
            let rows = self.db.search_albums_by_artist(artist_query, album_ids, limit)?;
            for row in rows {
                if seen.insert(row.album_source_id.clone()) {
                    let score = match_score(&row.artist_name, artist_query);
                    results.push(SearchResult {
                        id: format!("album-{}", row.album_source_id),
                        kind: SearchResultKind::Album,
                        album_source_id: row.album_source_id,
                        album_title: row.album_title,
                        artist_name: row.artist_name,
                        year: row.year,
                        album_art_path: row.art_url,
                        track_source_id: None,
                        track_title: None,
                        track_artist: None,
                        is_favourite: row.is_favourite,
                        score,
                    });
                }
            }
        }

        // Free text: search both artist name and album title
        if let Some(text) = query.free_text() {
            let rows = self.db.search_albums_by_artist_or_title(text, album_ids, limit)?;
            for row in rows {
                if seen.insert(row.album_source_id.clone()) {
                    let artist_score = match_score(&row.artist_name, text);
                    let title_score = match_score(&row.album_title, text);
                    let score = artist_score.min(title_score) + 0.1;
                    results.push(SearchResult {
                        id: format!("album-{}", row.album_source_id),
                        kind: SearchResultKind::Album,
                        album_source_id: row.album_source_id,
                        album_title: row.album_title,
                        artist_name: row.artist_name,
                        year: row.year,
                        album_art_path: row.art_url,
                        track_source_id: None,
                        track_title: None,
                        track_artist: None,
                        is_favourite: row.is_favourite,
                        score,
                    });
                }
            }
        }

        // Filters only (genre/year with no text) — list matching albums
        if query.free_text().is_none()
            && query.artist_filters().is_empty()
            && query.album_title_filters().is_empty()
            && album_ids.is_some()
        {
            let rows = self.db.search_albums_filtered(album_ids, limit)?;
            for row in rows {
                if seen.insert(row.album_source_id.clone()) {
                    results.push(SearchResult {
                        id: format!("album-{}", row.album_source_id),
                        kind: SearchResultKind::Album,
                        album_source_id: row.album_source_id,
                        album_title: row.album_title,
                        artist_name: row.artist_name,
                        year: row.year,
                        album_art_path: row.art_url,
                        track_source_id: None,
                        track_title: None,
                        track_artist: None,
                        is_favourite: row.is_favourite,
                        score: 0.0,
                    });
                }
            }
        }

        results.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());
        results.truncate(limit);
        Ok(results)
    }

    // --- Track Search ---

    fn search_tracks(
        &self,
        query: &ParsedQuery,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<SearchResult>, crate::cache::db::CacheError> {
        let mut results = Vec::new();

        for track_query in query.track_searches() {
            let partial = self.search_tracks_by_text(track_query, album_ids, limit)?;
            results.extend(partial);
        }

        results.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());
        results.truncate(limit);
        Ok(results)
    }

    fn search_tracks_by_text(
        &self,
        text: &str,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<SearchResult>, crate::cache::db::CacheError> {
        let mut seen = HashSet::<i64>::new();
        let mut results = Vec::new();

        let escaped = crate::util::escape_fts5(text);
        let fts_tokens: String = escaped
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| format!("\"{}\"*", t))
            .collect::<Vec<_>>()
            .join(" ");

        if !fts_tokens.is_empty() {
            let fts_results = self.db.search_tracks_enriched(&fts_tokens, album_ids, limit)?;
            for row in fts_results {
                if seen.insert(row.id) {
                    let score = match_score(&row.track_title, text);
                    results.push(SearchResult {
                        id: format!("track-{}", row.id),
                        kind: SearchResultKind::Track,
                        album_source_id: row.album_source_id,
                        album_title: row.album_title,
                        artist_name: row.artist_name,
                        year: None,
                        album_art_path: row.art_url,
                        track_source_id: Some(row.track_source_id),
                        track_title: Some(row.track_title),
                        track_artist: row.track_artist,
                        is_favourite: row.is_favourite,
                        score,
                    });
                }
            }
        }

        // Cross-field tokenised search: each token must match one of
        // (track title, album title, artist name), ANDed. FTS5 only
        // indexes the track title, so "radiohead karma" (tokens that span
        // artist + title) returns nothing from FTS5 alone. Run for every
        // multi-token query so we catch cross-field matches, and dedupe
        // against the FTS5 hits via `seen`. Add a small score penalty so
        // FTS5 title-only matches still sort first when both paths hit.
        if text.split_whitespace().filter(|t| !t.is_empty()).count() > 1 {
            let cross_results =
                self.db.search_tracks_by_tokens_cross_field(text, album_ids, limit)?;
            for row in cross_results {
                if seen.insert(row.id) {
                    let score = match_score(&row.track_title, text) + 0.05;
                    results.push(SearchResult {
                        id: format!("track-{}", row.id),
                        kind: SearchResultKind::Track,
                        album_source_id: row.album_source_id,
                        album_title: row.album_title,
                        artist_name: row.artist_name,
                        year: None,
                        album_art_path: row.art_url,
                        track_source_id: Some(row.track_source_id),
                        track_title: Some(row.track_title),
                        track_artist: row.track_artist,
                        is_favourite: row.is_favourite,
                        score,
                    });
                }
            }
        }

        // Fuzzy fallback if < 5 track results
        if results.len() < 5 {
            let fuzzy_results = self.fuzzy_track_search(text, album_ids, &seen)?;
            results.extend(fuzzy_results);
        }

        results.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());
        results.truncate(limit);
        Ok(results)
    }

    // --- Private ---

    fn resolve_album_constraints(
        &self,
        query: &ParsedQuery,
    ) -> Result<Option<HashSet<i64>>, crate::cache::db::CacheError> {
        let mut constrained_ids: Option<HashSet<i64>> = None;

        let genres = query.genre_filters();
        if !genres.is_empty() {
            let mut expanded_names = HashSet::new();
            for genre in &genres {
                if let Some(ref expander) = self.genre_expander {
                    if let Some(descendants) = expander.expand_genre(genre) {
                        expanded_names.extend(descendants);
                    } else {
                        expanded_names.insert(genre.to_string());
                    }
                } else {
                    expanded_names.insert(genre.to_string());
                }
            }
            let genre_album_ids = self
                .db
                .album_ids_for_genre_names(&expanded_names.into_iter().collect::<Vec<_>>())?;
            constrained_ids = Some(genre_album_ids);
        }

        for (field, op, value) in query.range_filters() {
            let matched_ids = match field {
                RangeField::Year => self.db.albums_by_year_range(op, value as i32)?,
                RangeField::Rating => self.db.albums_by_rating_range(op, value)?,
            };
            if let Some(existing) = constrained_ids {
                constrained_ids = Some(existing.intersection(&matched_ids).copied().collect());
            } else {
                constrained_ids = Some(matched_ids);
            }
        }

        if query.has_favourites_filter() {
            let fav_ids = self.db.album_ids_for_favourites()?;
            if let Some(existing) = constrained_ids {
                constrained_ids = Some(existing.intersection(&fav_ids).copied().collect());
            } else {
                constrained_ids = Some(fav_ids);
            }
        }

        Ok(constrained_ids)
    }

    fn fuzzy_track_search(
        &self,
        text: &str,
        album_ids: Option<&HashSet<i64>>,
        excluding: &HashSet<i64>,
    ) -> Result<Vec<SearchResult>, crate::cache::db::CacheError> {
        let candidates = self.db.search_candidates(album_ids, 5000)?;
        let query_lower = text.to_lowercase();

        let mut scored: Vec<(SearchResult, f64)> = Vec::new();

        for candidate in candidates {
            if excluding.contains(&candidate.id) {
                continue;
            }
            // Only match against track title — consistent with FTS5 (which also
            // only indexes titles). Album/artist name matching is already handled
            // by the LIKE queries on the albums table. Matching short album names
            // (e.g. "Woe") against longer queries inflates Jaro-Winkler scores and
            // pulls in every track on that album as a false positive.
            let similarity =
                strsim::jaro_winkler(&candidate.track_title.to_lowercase(), &query_lower);
            if similarity > 0.7 {
                scored.push((
                    SearchResult {
                        id: format!("track-{}", candidate.id),
                        kind: SearchResultKind::Track,
                        album_source_id: candidate.album_source_id,
                        album_title: candidate.album_title,
                        artist_name: candidate.artist_name,
                        year: None,
                        album_art_path: candidate.art_url,
                        track_source_id: Some(candidate.track_source_id),
                        track_title: Some(candidate.track_title),
                        track_artist: candidate.track_artist,
                        is_favourite: candidate.is_favourite,
                        score: 0.0,
                    },
                    similarity,
                ));
            }
        }

        // Sort descending by similarity (higher = better match)
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        Ok(scored
            .into_iter()
            .take(50)
            .map(|(mut result, similarity)| {
                // Prefix fuzzy scores with 0.5 to rank below FTS5 results
                result.score = 0.5 + (1.0 - similarity);
                result
            })
            .collect())
    }

    /// Return all internal album IDs matching the given query, with no limit.
    /// Used for "load into grid" — resolves constraints and text searches, then
    /// returns the union/intersection of matching album IDs.
    pub fn search_album_ids(
        &self,
        query: &ParsedQuery,
    ) -> Result<HashSet<i64>, crate::cache::db::CacheError> {
        if query.is_empty() {
            return Ok(HashSet::new());
        }

        let constraint_ids = self.resolve_album_constraints(query)?;
        if let Some(ref ids) = constraint_ids {
            if ids.is_empty() {
                return Ok(HashSet::new());
            }
        }

        let has_text_search = query.free_text().is_some()
            || !query.artist_filters().is_empty()
            || !query.album_title_filters().is_empty();

        if !has_text_search {
            // Constraint-only query (genre/year/rating/fav) — return the constraint set
            return Ok(constraint_ids.unwrap_or_default());
        }

        // Collect album IDs from text searches
        let mut text_ids = HashSet::new();

        for title_query in query.album_title_filters() {
            let ids = self
                .db
                .album_ids_by_title(title_query, constraint_ids.as_ref())?;
            text_ids.extend(ids);
        }

        for artist_query in query.artist_filters() {
            let ids = self
                .db
                .album_ids_by_artist(artist_query, constraint_ids.as_ref())?;
            text_ids.extend(ids);
        }

        if let Some(text) = query.free_text() {
            let ids = self
                .db
                .album_ids_by_artist_or_title(text, constraint_ids.as_ref())?;
            text_ids.extend(ids);
        }

        Ok(text_ids)
    }
}

/// Score a match: exact = 0.0, starts-with = 0.02, contains = 0.05
fn match_score(value: &str, query: &str) -> f64 {
    let v = value.to_lowercase();
    let q = query.to_lowercase();
    if v == q {
        0.0
    } else if v.starts_with(&q) {
        0.02
    } else {
        0.05
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db::{AlbumUpsertRow, TrackUpsertRow};
    use crate::search::parser::QueryParser;

    fn setup() -> (Arc<CacheDatabase>, SearchEngine) {
        let db = Arc::new(CacheDatabase::open_in_memory().unwrap());
        let engine = SearchEngine::new(db.clone(), None);
        seed_test_data(&db);
        (db, engine)
    }

    fn seed_test_data(db: &CacheDatabase) {
        let artist_map = db
            .batch_upsert_artists(&[
                ("Radiohead".into(), None, "artist-1".into(), None, None, None),
                ("Slayer".into(), None, "artist-2".into(), None, None, None),
            ])
            .unwrap();
        let radiohead_id = *artist_map.get("artist-1").unwrap();
        let slayer_id = *artist_map.get("artist-2").unwrap();

        let album_map = db
            .batch_upsert_albums(&[
                AlbumUpsertRow {
                    title: "OK Computer".into(),
                    artist_id: radiohead_id,
                    year: Some(1997),
                    source_id: "album-1".into(),
                    art_url: None,
                    updated_at: None,
                    added_at: None,
                    last_viewed_at: None,
                    first_genre: None,
                },
                AlbumUpsertRow {
                    title: "Reign in Blood".into(),
                    artist_id: slayer_id,
                    year: Some(1986),
                    source_id: "album-2".into(),
                    art_url: None,
                    updated_at: None,
                    added_at: None,
                    last_viewed_at: None,
                    first_genre: None,
                },
                AlbumUpsertRow {
                    title: "Kid A".into(),
                    artist_id: radiohead_id,
                    year: Some(2000),
                    source_id: "album-3".into(),
                    art_url: None,
                    updated_at: None,
                    added_at: None,
                    last_viewed_at: None,
                    first_genre: None,
                },
            ])
            .unwrap();
        let ok_computer_id = *album_map.get("album-1").unwrap();
        let reign_id = *album_map.get("album-2").unwrap();
        let kid_a_id = *album_map.get("album-3").unwrap();

        db.batch_upsert_tracks(&[
            TrackUpsertRow {
                title: "Paranoid Android".into(),
                album_id: ok_computer_id,
                artist_id: radiohead_id,
                track_number: Some(1),
                disc_number: Some(1),
                duration_ms: Some(384000),
                source_id: "track-1".into(),
                codec: Some("flac".into()),
                part_key: None,
                stream_id: None,
                user_rating: None,
                bitrate: None,
                track_artist: None,
                updated_at: None,
            },
            TrackUpsertRow {
                title: "Karma Police".into(),
                album_id: ok_computer_id,
                artist_id: radiohead_id,
                track_number: Some(2),
                disc_number: Some(1),
                duration_ms: Some(264000),
                source_id: "track-2".into(),
                codec: Some("flac".into()),
                part_key: None,
                stream_id: None,
                user_rating: None,
                bitrate: None,
                track_artist: None,
                updated_at: None,
            },
            TrackUpsertRow {
                title: "Angel of Death".into(),
                album_id: reign_id,
                artist_id: slayer_id,
                track_number: Some(1),
                disc_number: Some(1),
                duration_ms: Some(294000),
                source_id: "track-3".into(),
                codec: Some("flac".into()),
                part_key: None,
                stream_id: None,
                user_rating: None,
                bitrate: None,
                track_artist: None,
                updated_at: None,
            },
            TrackUpsertRow {
                title: "Raining Blood".into(),
                album_id: reign_id,
                artist_id: slayer_id,
                track_number: Some(2),
                disc_number: Some(1),
                duration_ms: Some(252000),
                source_id: "track-4".into(),
                codec: Some("flac".into()),
                part_key: None,
                stream_id: None,
                user_rating: None,
                bitrate: None,
                track_artist: None,
                updated_at: None,
            },
            TrackUpsertRow {
                title: "Everything In Its Right Place".into(),
                album_id: kid_a_id,
                artist_id: radiohead_id,
                track_number: Some(1),
                disc_number: Some(1),
                duration_ms: Some(250000),
                source_id: "track-5".into(),
                codec: Some("flac".into()),
                part_key: None,
                stream_id: None,
                user_rating: None,
                bitrate: None,
                track_artist: None,
                updated_at: None,
            },
        ])
        .unwrap();

        let rock_id = db.upsert_genre("Rock").unwrap();
        let metal_id = db.upsert_genre("Metal").unwrap();
        let electronic_id = db.upsert_genre("Electronic").unwrap();

        db.set_album_genres(ok_computer_id, &[rock_id]).unwrap();
        db.set_album_genres(reign_id, &[metal_id]).unwrap();
        db.set_album_genres(kid_a_id, &[rock_id, electronic_id]).unwrap();
    }

    // --- Album Search Tests ---

    #[test]
    fn test_free_text_search_returns_albums_and_tracks() {
        let (_db, engine) = setup();
        // "radiohead" matches artist name via LIKE → album results
        let q = QueryParser::parse("radiohead");
        let results = engine.search(&q, 100).unwrap();
        let albums: Vec<_> = results.iter().filter(|r| r.kind == SearchResultKind::Album).collect();
        assert!(!albums.is_empty(), "Should have album results");
        assert!(albums.iter().all(|r| r.artist_name == "Radiohead"));

        // "paranoid" matches track title via FTS5 → track results
        let q2 = QueryParser::parse("paranoid");
        let results2 = engine.search(&q2, 100).unwrap();
        let tracks: Vec<_> = results2.iter().filter(|r| r.kind == SearchResultKind::Track).collect();
        assert!(!tracks.is_empty(), "Should have track results for title match");
        assert!(tracks.iter().any(|r| r.track_title.as_deref() == Some("Paranoid Android")));
    }

    #[test]
    fn test_artist_filter_returns_albums() {
        let (_db, engine) = setup();
        let q = QueryParser::parse("@slayer");
        let results = engine.search(&q, 100).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.kind == SearchResultKind::Album));
        assert!(results.iter().all(|r| r.artist_name == "Slayer"));
    }

    #[test]
    fn test_album_title_filter() {
        let (_db, engine) = setup();
        let q = QueryParser::parse("%ok computer");
        let results = engine.search(&q, 100).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.kind == SearchResultKind::Album));
        assert_eq!(results[0].album_title, "OK Computer");
    }

    #[test]
    fn test_genre_filter_returns_albums() {
        let (_db, engine) = setup();
        let q = QueryParser::parse("/rock");
        let results = engine.search(&q, 100).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.kind == SearchResultKind::Album));
        let titles: HashSet<_> = results.iter().map(|r| r.album_title.as_str()).collect();
        assert!(titles.contains("OK Computer"));
        assert!(titles.contains("Kid A"));
        assert!(!titles.contains("Reign in Blood"));
    }

    #[test]
    fn test_genre_filter_expands_hierarchy() {
        let db = Arc::new(CacheDatabase::open_in_memory().unwrap());
        seed_test_data(&db);

        // Create a simple genre expander that makes "Electronic" a child of "Rock"
        struct TestExpander;
        impl GenreExpander for TestExpander {
            fn expand_genre(&self, name: &str) -> Option<HashSet<String>> {
                if name.eq_ignore_ascii_case("rock") {
                    let mut set = HashSet::new();
                    set.insert("Rock".to_string());
                    set.insert("Electronic".to_string());
                    Some(set)
                } else {
                    None
                }
            }
        }

        let engine = SearchEngine::new(db, Some(Arc::new(TestExpander)));
        let q = QueryParser::parse("/rock");
        let results = engine.search(&q, 100).unwrap();
        let titles: HashSet<_> = results.iter().map(|r| r.album_title.as_str()).collect();
        assert!(titles.contains("OK Computer"), "Should include Rock-tagged album");
        assert!(titles.contains("Kid A"), "Should include Electronic-tagged album (child of Rock)");
        assert!(!titles.contains("Reign in Blood"), "Should not include Metal-tagged album");
    }

    #[test]
    fn test_year_range_filter() {
        let (_db, engine) = setup();
        let q = QueryParser::parse("year:>1999");
        let results = engine.search(&q, 100).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.album_title == "Kid A"));
    }

    #[test]
    fn test_combined_filters() {
        let (_db, engine) = setup();
        let q = QueryParser::parse("/rock AND year:>1999");
        let results = engine.search(&q, 100).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.album_title == "Kid A"));
    }

    // --- Track Search Tests ---

    #[test]
    fn test_track_search_returns_tracks_only() {
        let (_db, engine) = setup();
        let q = QueryParser::parse("!paranoid");
        let results = engine.search(&q, 100).unwrap();
        let tracks: Vec<_> = results.iter().filter(|r| r.kind == SearchResultKind::Track).collect();
        assert!(!tracks.is_empty());
        assert_eq!(tracks[0].track_title.as_deref(), Some("Paranoid Android"));
    }

    #[test]
    fn test_track_search_fuzzy_fallback() {
        let (_db, engine) = setup();
        let q = QueryParser::parse("!paranoyd");
        let results = engine.search(&q, 100).unwrap();
        let tracks: Vec<_> = results.iter().filter(|r| r.kind == SearchResultKind::Track).collect();
        assert!(!tracks.is_empty(), "Fuzzy should find 'Paranoid Android' for typo 'paranoyd'");
    }

    #[test]
    fn test_free_text_albums_appear_before_tracks() {
        let (_db, engine) = setup();
        let q = QueryParser::parse("radiohead");
        let results = engine.search(&q, 100).unwrap();
        let first_track_idx = results.iter().position(|r| r.kind == SearchResultKind::Track);
        let last_album_idx = results.iter().rposition(|r| r.kind == SearchResultKind::Album);
        if let (Some(ti), Some(ai)) = (first_track_idx, last_album_idx) {
            assert!(ti > ai, "All albums should appear before any track");
        }
    }

    #[test]
    fn test_free_text_gibberish_returns_empty() {
        let (_db, engine) = setup();
        let q = QueryParser::parse("zzzznonexistent");
        let results = engine.search(&q, 100).unwrap();
        assert!(results.is_empty(), "No results for gibberish query");
    }

    #[test]
    fn test_free_text_albums_capped_at_five() {
        let (_db, engine) = setup();
        let q = QueryParser::parse("radiohead");
        let results = engine.search(&q, 100).unwrap();
        let albums: Vec<_> = results.iter().filter(|r| r.kind == SearchResultKind::Album).collect();
        assert!(albums.len() <= 5);
    }

    #[test]
    fn test_empty_query() {
        let (_db, engine) = setup();
        let q = QueryParser::parse("");
        let results = engine.search(&q, 100).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_free_text_cross_field_artist_plus_track_title() {
        // "radiohead AND karma" merges to FreeText("radiohead karma"). FTS5
        // alone can't match this (tracks_fts only indexes title, and no
        // title has both tokens). The cross-field search should catch it
        // via artist=Radiohead + track title=Karma Police.
        let (_db, engine) = setup();
        let q = QueryParser::parse("radiohead AND karma");
        let results = engine.search(&q, 100).unwrap();
        let tracks: Vec<_> = results
            .iter()
            .filter(|r| r.kind == SearchResultKind::Track)
            .collect();
        assert!(
            tracks
                .iter()
                .any(|r| r.track_title.as_deref() == Some("Karma Police")),
            "cross-field search should find Karma Police by Radiohead"
        );
    }

    #[test]
    fn test_free_text_cross_field_artist_plus_album_title() {
        // "radiohead ok" should find OK Computer by Radiohead via the
        // tokenised album LIKE search (artist + album title across tokens).
        let (_db, engine) = setup();
        let q = QueryParser::parse("radiohead ok");
        let results = engine.search(&q, 100).unwrap();
        let albums: Vec<_> = results
            .iter()
            .filter(|r| r.kind == SearchResultKind::Album)
            .collect();
        assert!(
            albums.iter().any(|r| r.album_title == "OK Computer"),
            "token-AND album search should find OK Computer"
        );
    }
}
