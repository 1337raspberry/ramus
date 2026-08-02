use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::cache::db::{
    AlbumSearchRow, ArtistSearchRow, CacheDatabase, CacheError, TrackSearchRow,
};
use crate::models::{
    RangeField, SearchAlbumResult, SearchArtistResult, SearchGenreResult, SearchResponse,
    SearchSection, SearchTrackResult,
};
use crate::search::parser::ParsedQuery;
use crate::util::{fold_diacritics, is_lossless_codec};

/// Expand a genre name into all descendant genre names. Implemented by
/// `GenreMapper` in the genre module.
pub trait GenreExpander: Send + Sync {
    fn expand_genre(&self, name: &str) -> Option<HashSet<String>>;
}

// Score ladder (lower = better). Exact and prefix matches on the entity's
// own name dominate; token and fuzzy matches trail.
const SCORE_EXACT: f64 = 0.0;
const SCORE_PREFIX: f64 = 0.02;
const SCORE_WORD_PREFIX: f64 = 0.04;
const SCORE_CONTAINS: f64 = 0.05;
const SCORE_TOKEN_AND: f64 = 0.07;

/// An album matching via its artist's name scores slightly worse than the
/// same match on its own title, so title hits sort first.
const ARTIST_NAME_FIELD_PENALTY: f64 = 0.03;
/// Albums seeded from a matched artist entity (including fuzzy-matched
/// artists whose albums a plain title search would miss).
const ARTIST_ALBUM_SEED_PENALTY: f64 = 0.05;
/// Tracks pulled in purely because their artist matched exactly.
const ARTIST_TRACK_FILL_PENALTY: f64 = 0.12;
/// Genres rank behind an equally-good artist/album/track match.
const GENRE_SECTION_PENALTY: f64 = 0.10;

/// Matched artists at or below this score seed their albums into the
/// album section (covers exact through reasonable fuzzy matches).
const ARTIST_SEED_MAX_SCORE: f64 = 0.8;
/// How many matched artists may seed albums (best-scored first).
const ARTIST_SEED_MAX_ARTISTS: usize = 3;

const FUZZY_SIM_THRESHOLD: f64 = 0.75;
/// Queries shorter than this never fuzzy-match (too noisy).
const FUZZY_MIN_QUERY_LEN: usize = 4;

const ALBUM_CANDIDATE_CAP: usize = 20_000;
const TRACK_FUZZY_CANDIDATE_CAP: usize = 5_000;

/// A query string pre-folded for comparison, with its whitespace tokens.
struct FoldedQuery {
    folded: String,
    tokens: Vec<String>,
}

impl FoldedQuery {
    fn new(text: &str) -> Self {
        let folded = fold_diacritics(text.trim());
        let tokens = folded.split_whitespace().map(str::to_string).collect();
        Self { folded, tokens }
    }

    fn is_empty(&self) -> bool {
        self.folded.is_empty()
    }
}

/// Score `value` against the query on the standard ladder. `None` = no
/// match. Fuzzy is optional because some surfaces (genres) want only
/// literal matches.
fn ladder_score(value: &str, q: &FoldedQuery, allow_fuzzy: bool) -> Option<f64> {
    if q.is_empty() {
        return None;
    }
    let v = fold_diacritics(value);
    if v == q.folded {
        return Some(SCORE_EXACT);
    }
    if v.starts_with(&q.folded) {
        return Some(SCORE_PREFIX);
    }
    if v.split_whitespace().any(|w| w.starts_with(q.folded.as_str())) {
        return Some(SCORE_WORD_PREFIX);
    }
    if v.contains(&q.folded) {
        return Some(SCORE_CONTAINS);
    }
    if q.tokens.len() > 1 && q.tokens.iter().all(|t| v.contains(t.as_str())) {
        return Some(SCORE_TOKEN_AND);
    }
    if allow_fuzzy && q.folded.len() >= FUZZY_MIN_QUERY_LEN {
        let sim = fuzzy_sim(&v, &q.folded);
        if sim > FUZZY_SIM_THRESHOLD {
            return Some(0.5 + (1.0 - sim));
        }
    }
    None
}

/// Token-AND across multiple fields: every query token must appear in the
/// concatenation of the fields. Catches "my c" → title "My Journey…" +
/// artist "Violet Cold".
fn cross_field_token_score(fields: &[&str], q: &FoldedQuery) -> Option<f64> {
    if q.tokens.len() < 2 {
        return None;
    }
    let combined = fields
        .iter()
        .map(|f| fold_diacritics(f))
        .collect::<Vec<_>>()
        .join(" ");
    if q.tokens.iter().all(|t| combined.contains(t.as_str())) {
        Some(SCORE_TOKEN_AND)
    } else {
        None
    }
}

fn merge_min(best: &mut Option<f64>, candidate: Option<f64>) {
    if let Some(c) = candidate {
        *best = Some(best.map_or(c, |b: f64| b.min(c)));
    }
}

fn album_row_to_result(row: &AlbumSearchRow, score: f64) -> SearchAlbumResult {
    SearchAlbumResult {
        source_id: row.album_source_id.clone(),
        title: row.album_title.clone(),
        artist_name: row.artist_name.clone(),
        year: row.year,
        art_url: row.art_url.clone(),
        rating: row.rating,
        quality: None,
        is_favourite: row.is_favourite,
        score,
    }
}

fn track_row_to_result(row: &TrackSearchRow, score: f64) -> SearchTrackResult {
    SearchTrackResult {
        source_id: row.track_source_id.clone(),
        title: row.track_title.clone(),
        display_artist: row
            .track_artist
            .clone()
            .unwrap_or_else(|| row.artist_name.clone()),
        album_source_id: row.album_source_id.clone(),
        album_title: row.album_title.clone(),
        art_url: row.art_url.clone(),
        rating: row.user_rating,
        is_favourite: row.is_favourite,
        score,
    }
}

/// "FLAC" for lossless codecs, "MP3 320" (codec + kbps) for lossy.
fn format_quality(codec: Option<&str>, bitrate: Option<i64>) -> Option<String> {
    let codec = codec?;
    if codec.is_empty() {
        return None;
    }
    let upper = codec.to_uppercase();
    if is_lossless_codec(codec) {
        Some(upper)
    } else {
        match bitrate {
            Some(kbps) if kbps > 0 => Some(format!("{} {}", upper, kbps)),
            _ => Some(upper),
        }
    }
}

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

    /// Execute a parsed query into ordered result sections. Sections are
    /// ordered by their best (lowest) item score; each section is sorted
    /// best-first and capped at `section_limit` items. Empty sections are
    /// omitted.
    pub fn search_sectioned(
        &self,
        query: &ParsedQuery,
        section_limit: usize,
    ) -> Result<SearchResponse, CacheError> {
        if query.is_empty() || section_limit == 0 {
            return Ok(SearchResponse { sections: Vec::new() });
        }

        let album_ids = self.resolve_album_constraints(query)?;
        if let Some(ref ids) = album_ids {
            if ids.is_empty() {
                return Ok(SearchResponse { sections: Vec::new() });
            }
        }
        // Operator constraints (genre/year/rating/fav/collection) scope
        // albums and tracks; artist and genre entities have no album scope,
        // so those sections are skipped for constrained queries.
        let constrained = album_ids.is_some();
        let free = query.free_text();

        // ---- Artists ----
        let mut matched_artists: Vec<(ArtistSearchRow, f64)> = Vec::new();
        if !constrained {
            let mut artist_queries: Vec<FoldedQuery> = query
                .artist_filters()
                .iter()
                .map(|s| FoldedQuery::new(s))
                .collect();
            if let Some(text) = free {
                artist_queries.push(FoldedQuery::new(text));
            }
            artist_queries.retain(|q| !q.is_empty());
            if !artist_queries.is_empty() {
                for row in self.db.artist_search_candidates()? {
                    let mut best: Option<f64> = None;
                    for q in &artist_queries {
                        merge_min(&mut best, ladder_score(&row.name, q, true));
                    }
                    if let Some(score) = best {
                        matched_artists.push((row, score));
                    }
                }
                matched_artists
                    .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap().then(a.0.name.cmp(&b.0.name)));
            }
        }
        let artist_results: Vec<SearchArtistResult> = matched_artists
            .iter()
            .take(section_limit)
            .map(|(row, score)| SearchArtistResult {
                source_id: row.source_id.clone(),
                name: row.name.clone(),
                art_url: row.art_url.clone(),
                album_count: row.album_count,
                score: *score,
            })
            .collect();

        // ---- Albums ----
        let mut album_pool: HashMap<String, SearchAlbumResult> = HashMap::new();
        let upsert_album = |pool: &mut HashMap<String, SearchAlbumResult>,
                                row: &AlbumSearchRow,
                                score: f64| {
            pool.entry(row.album_source_id.clone())
                .and_modify(|existing| {
                    if score < existing.score {
                        existing.score = score;
                    }
                })
                .or_insert_with(|| album_row_to_result(row, score));
        };

        let has_album_text = free.is_some()
            || !query.artist_filters().is_empty()
            || !query.album_title_filters().is_empty();

        if has_album_text {
            let candidates = self
                .db
                .search_album_candidates(album_ids.as_ref(), ALBUM_CANDIDATE_CAP)?;
            let free_q = free.map(FoldedQuery::new);
            let artist_qs: Vec<FoldedQuery> = query
                .artist_filters()
                .iter()
                .map(|s| FoldedQuery::new(s))
                .collect();
            let title_qs: Vec<FoldedQuery> = query
                .album_title_filters()
                .iter()
                .map(|s| FoldedQuery::new(s))
                .collect();

            for row in &candidates {
                let mut best: Option<f64> = None;
                if let Some(q) = &free_q {
                    merge_min(&mut best, ladder_score(&row.album_title, q, true));
                    merge_min(
                        &mut best,
                        ladder_score(&row.artist_name, q, true)
                            .map(|s| s + ARTIST_NAME_FIELD_PENALTY),
                    );
                    merge_min(
                        &mut best,
                        cross_field_token_score(&[&row.album_title, &row.artist_name], q),
                    );
                }
                for q in &artist_qs {
                    merge_min(&mut best, ladder_score(&row.artist_name, q, true));
                }
                for q in &title_qs {
                    merge_min(&mut best, ladder_score(&row.album_title, q, true));
                }
                if let Some(score) = best {
                    upsert_album(&mut album_pool, row, score);
                }
            }
        }

        // Albums of matched artists join even when their titles match
        // nothing — a fuzzy-matched artist's discography is usually what
        // the typo was after.
        for (artist, artist_score) in matched_artists
            .iter()
            .filter(|(_, s)| *s <= ARTIST_SEED_MAX_SCORE)
            .take(ARTIST_SEED_MAX_ARTISTS)
        {
            let rows = self.db.albums_for_artist_id(artist.id, section_limit)?;
            for row in &rows {
                upsert_album(&mut album_pool, row, artist_score + ARTIST_ALBUM_SEED_PENALTY);
            }
        }

        // Filter-only query (genre/year/favourites/collection): list
        // matching albums in the DB's (random) order.
        if !has_album_text && album_ids.is_some() {
            let rows = self.db.search_albums_filtered(album_ids.as_ref(), section_limit)?;
            for (i, row) in rows.iter().enumerate() {
                // Tiny increments preserve the randomized order through
                // the score sort below.
                upsert_album(&mut album_pool, row, i as f64 * 1e-9);
            }
        }

        let mut albums: Vec<SearchAlbumResult> = album_pool.into_values().collect();
        albums.sort_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap()
                .then_with(|| a.title.cmp(&b.title))
        });
        albums.truncate(section_limit);

        if !albums.is_empty() {
            let ids: Vec<String> = albums.iter().map(|a| a.source_id.clone()).collect();
            let info = self.db.album_codec_info(&ids)?;
            for album in &mut albums {
                if let Some((codec, bitrate)) = info.get(&album.source_id) {
                    album.quality = format_quality(codec.as_deref(), *bitrate);
                }
            }
        }

        // ---- Tracks ----
        let mut track_pool: HashMap<i64, SearchTrackResult> = HashMap::new();
        if query.has_track_search() {
            for text in query.track_searches() {
                self.collect_tracks_for_text(text, album_ids.as_ref(), section_limit, true, &mut track_pool)?;
            }
        } else if let Some(text) = free {
            self.collect_tracks_for_text(text, album_ids.as_ref(), section_limit, false, &mut track_pool)?;

            // A folded-exact artist match fills the track section with
            // that artist's best tracks (mirrors searching a band name and
            // expecting songs, not just albums).
            if let Some((artist, artist_score)) = matched_artists.first() {
                if *artist_score == SCORE_EXACT && !constrained {
                    let rows = self.db.tracks_for_artist_id(artist.id, section_limit)?;
                    for row in &rows {
                        let score = artist_score + ARTIST_TRACK_FILL_PENALTY;
                        track_pool
                            .entry(row.id)
                            .and_modify(|existing| {
                                if score < existing.score {
                                    existing.score = score;
                                }
                            })
                            .or_insert_with(|| track_row_to_result(row, score));
                    }
                }
            }
        }

        let mut tracks: Vec<SearchTrackResult> = track_pool.into_values().collect();
        tracks.sort_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap()
                .then_with(|| a.title.cmp(&b.title))
        });
        tracks.truncate(section_limit);
        // Relative junk cutoff: a strong best match makes weak fuzzy tails
        // look silly next to it.
        if let Some(best_score) = tracks.first().map(|r| r.score) {
            let max_acceptable = if best_score < 0.1 { 0.3 } else { 0.7 };
            tracks.retain(|r| r.score <= max_acceptable);
        }

        // ---- Genres ----
        let mut genres: Vec<SearchGenreResult> = Vec::new();
        if !constrained {
            if let Some(text) = free {
                let q = FoldedQuery::new(text);
                for row in self.db.genre_search_candidates()? {
                    // Literal matches only — fuzzy genre suggestions are
                    // more confusing than helpful.
                    if let Some(score) = ladder_score(&row.name, &q, false) {
                        genres.push(SearchGenreResult {
                            name: row.name,
                            album_count: row.album_count,
                            score: score + GENRE_SECTION_PENALTY,
                        });
                    }
                }
                genres.sort_by(|a, b| {
                    a.score
                        .partial_cmp(&b.score)
                        .unwrap()
                        .then_with(|| a.name.cmp(&b.name))
                });
                genres.truncate(section_limit);
            }
        }

        // ---- Assemble: order sections by best score, stable tie-break ----
        let mut ordered: Vec<(f64, u8, SearchSection)> = Vec::new();
        if !artist_results.is_empty() {
            ordered.push((artist_results[0].score, 0, SearchSection::Artists(artist_results)));
        }
        if !albums.is_empty() {
            ordered.push((albums[0].score, 1, SearchSection::Albums(albums)));
        }
        if !tracks.is_empty() {
            ordered.push((tracks[0].score, 2, SearchSection::Tracks(tracks)));
        }
        if !genres.is_empty() {
            ordered.push((genres[0].score, 3, SearchSection::Genres(genres)));
        }
        ordered.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));

        Ok(SearchResponse {
            sections: ordered.into_iter().map(|(_, _, s)| s).collect(),
        })
    }

    /// Gather track matches for one query text into `pool` (keyed by
    /// internal track id, keeping the best score). Three passes: FTS5 on
    /// title (diacritic-folded by unicode61), cross-field token SQL for
    /// multi-token queries, and a fuzzy sweep when direct hits are sparse.
    ///
    /// `cross_field_fuzzy` extends the fuzzy sweep to album/artist fields.
    /// Wanted for explicit track searches (`!`), where tracks are the only
    /// section; unwanted for free text, where a track matching purely via
    /// its album or artist is already represented by that album/artist's
    /// own section row.
    fn collect_tracks_for_text(
        &self,
        text: &str,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
        cross_field_fuzzy: bool,
        pool: &mut HashMap<i64, SearchTrackResult>,
    ) -> Result<(), CacheError> {
        let q = FoldedQuery::new(text);
        if q.is_empty() {
            return Ok(());
        }
        let upsert = |pool: &mut HashMap<i64, SearchTrackResult>,
                          row: &TrackSearchRow,
                          score: f64| {
            pool.entry(row.id)
                .and_modify(|existing| {
                    if score < existing.score {
                        existing.score = score;
                    }
                })
                .or_insert_with(|| track_row_to_result(row, score));
        };

        let escaped = crate::util::escape_fts5(text);
        let fts_tokens: String = escaped
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| format!("\"{}\"*", t))
            .collect::<Vec<_>>()
            .join(" ");
        if !fts_tokens.is_empty() {
            for row in self.db.search_tracks_enriched(&fts_tokens, album_ids, limit)? {
                // FTS guarantees all tokens hit the title; if the ladder
                // can't see a contiguous match, it was a scattered
                // multi-token hit.
                let score = ladder_score(&row.track_title, &q, false).unwrap_or(SCORE_TOKEN_AND);
                upsert(pool, &row, score);
            }
        }

        if q.tokens.len() > 1 {
            for row in self.db.search_tracks_by_tokens_cross_field(text, album_ids, limit)? {
                let score = ladder_score(&row.track_title, &q, false)
                    .unwrap_or(f64::MAX)
                    .min(SCORE_TOKEN_AND);
                upsert(pool, &row, score);
            }
        }

        // Fuzzy fallback when direct matches are sparse.
        if pool.len() < 5 && q.folded.len() >= FUZZY_MIN_QUERY_LEN {
            let candidates = self.db.search_candidates(album_ids, TRACK_FUZZY_CANDIDATE_CAP)?;
            for row in &candidates {
                if pool.contains_key(&row.id) {
                    continue;
                }
                let title_sim = fuzzy_sim(&fold_diacritics(&row.track_title), &q.folded);
                let similarity = if cross_field_fuzzy {
                    let album_sim = fuzzy_sim(&fold_diacritics(&row.album_title), &q.folded);
                    let artist_sim = fuzzy_sim(&fold_diacritics(&row.artist_name), &q.folded);
                    title_sim.max(album_sim).max(artist_sim)
                } else {
                    title_sim
                };
                if similarity > FUZZY_SIM_THRESHOLD {
                    let cross_field_penalty = if title_sim < similarity { 0.02 } else { 0.0 };
                    let score = 0.5 + (1.0 - similarity) + cross_field_penalty;
                    upsert(pool, row, score);
                }
            }
        }

        Ok(())
    }

    fn resolve_album_constraints(
        &self,
        query: &ParsedQuery,
    ) -> Result<Option<HashSet<i64>>, CacheError> {
        let mut constrained_ids: Option<HashSet<i64>> = None;

        let genres = query.genre_filters();
        if !genres.is_empty() {
            let mut expanded_names = HashSet::new();
            for genre in &genres {
                let expansion = self
                    .genre_expander
                    .as_ref()
                    .and_then(|expander| expander.expand_genre(genre));
                // Guard against bad fuzzy matches: if the expansion doesn't
                // contain the original name (case-insensitive), the mapper
                // fuzzy-matched to a different family (e.g. "blackgaze" →
                // Black Metal). Fall back to the raw DB name so "Other"
                // genres still resolve. Mirrors the guard in
                // `commands::library::get_albums_for_genre`.
                let genre_lower = genre.to_lowercase();
                match expansion {
                    Some(descendants)
                        if descendants.iter().any(|n| n.to_lowercase() == genre_lower) =>
                    {
                        expanded_names.extend(descendants);
                    }
                    _ => {
                        expanded_names.insert(genre.to_string());
                    }
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

        let collections = query.collection_filters();
        if !collections.is_empty() {
            let names: Vec<String> = collections.iter().map(|s| s.to_string()).collect();
            let col_ids = self.db.album_ids_for_collection_names(&names)?;
            if let Some(existing) = constrained_ids {
                constrained_ids = Some(existing.intersection(&col_ids).copied().collect());
            } else {
                constrained_ids = Some(col_ids);
            }
        }

        Ok(constrained_ids)
    }

    /// All internal album IDs matching the given query, with no limit.
    /// Resolves constraints and text searches, then returns the union/
    /// intersection of matching album IDs.
    pub fn search_album_ids(
        &self,
        query: &ParsedQuery,
    ) -> Result<HashSet<i64>, CacheError> {
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
            // Constraint-only query (genre/year/rating/fav).
            return Ok(constraint_ids.unwrap_or_default());
        }

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

fn strip_punctuation(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect()
}

/// NDL similarity checking both the full string and individual words,
/// with punctuation stripped. For multi-word values like "Paranoid Android",
/// a query of "paranoyd" matches the word "Paranoid" rather than requiring
/// similarity against the entire title.
fn fuzzy_sim(value: &str, query: &str) -> f64 {
    let clean = strip_punctuation(value);
    let full = strsim::normalized_damerau_levenshtein(&clean, query);
    let best_word = clean
        .split_whitespace()
        .map(|w| strsim::normalized_damerau_levenshtein(w, query))
        .fold(0.0_f64, f64::max);
    full.max(best_word)
}

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

    fn run(engine: &SearchEngine, q: &str) -> SearchResponse {
        engine
            .search_sectioned(&QueryParser::parse(q), 20)
            .unwrap()
    }

    fn artists_of(resp: &SearchResponse) -> &[SearchArtistResult] {
        resp.sections
            .iter()
            .find_map(|s| match s {
                SearchSection::Artists(v) => Some(v.as_slice()),
                _ => None,
            })
            .unwrap_or(&[])
    }

    fn albums_of(resp: &SearchResponse) -> &[SearchAlbumResult] {
        resp.sections
            .iter()
            .find_map(|s| match s {
                SearchSection::Albums(v) => Some(v.as_slice()),
                _ => None,
            })
            .unwrap_or(&[])
    }

    fn tracks_of(resp: &SearchResponse) -> &[SearchTrackResult] {
        resp.sections
            .iter()
            .find_map(|s| match s {
                SearchSection::Tracks(v) => Some(v.as_slice()),
                _ => None,
            })
            .unwrap_or(&[])
    }

    fn genres_of(resp: &SearchResponse) -> &[SearchGenreResult] {
        resp.sections
            .iter()
            .find_map(|s| match s {
                SearchSection::Genres(v) => Some(v.as_slice()),
                _ => None,
            })
            .unwrap_or(&[])
    }

    fn section_kinds(resp: &SearchResponse) -> Vec<&'static str> {
        resp.sections
            .iter()
            .map(|s| match s {
                SearchSection::Artists(_) => "artists",
                SearchSection::Albums(_) => "albums",
                SearchSection::Tracks(_) => "tracks",
                SearchSection::Genres(_) => "genres",
            })
            .collect()
    }

    fn album_titles(resp: &SearchResponse) -> Vec<String> {
        albums_of(resp).iter().map(|a| a.title.clone()).collect()
    }

    fn track_titles(resp: &SearchResponse) -> Vec<String> {
        tracks_of(resp).iter().map(|t| t.title.clone()).collect()
    }

    fn seed_test_data(db: &CacheDatabase) {
        let artist_map = db
            .batch_upsert_artists(&[
                ("Radiohead".into(), None, "artist-1".into(), None, None, None, None),
                ("Slayer".into(), None, "artist-2".into(), None, None, None, None),
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
                    first_collection: None,
                    view_count: None,
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
                    first_collection: None,
                    view_count: None,
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
                    first_collection: None,
                    view_count: None,
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
                file_size_bytes: None,
                rating_count: None,
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
                file_size_bytes: None,
                rating_count: None,
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
                file_size_bytes: None,
                rating_count: None,
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
                file_size_bytes: None,
                rating_count: None,
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
                file_size_bytes: None,
                rating_count: None,
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

    /// Seed one extra artist + album (+ optional tracks) for tests that
    /// need data outside the shared fixture.
    fn seed_artist_album(
        db: &CacheDatabase,
        artist: &str,
        artist_key: &str,
        album: &str,
        album_key: &str,
        tracks: &[&str],
    ) -> (i64, i64) {
        let artist_id = *db
            .batch_upsert_artists(&[(artist.into(), None, artist_key.into(), None, None, None, None)])
            .unwrap()
            .get(artist_key)
            .unwrap();
        let album_id = *db
            .batch_upsert_albums(&[AlbumUpsertRow {
                title: album.into(),
                artist_id,
                year: Some(2010),
                source_id: album_key.into(),
                art_url: None,
                updated_at: None,
                added_at: None,
                last_viewed_at: None,
                first_genre: None,
                first_collection: None,
                view_count: None,
            }])
            .unwrap()
            .get(album_key)
            .unwrap();
        let rows: Vec<TrackUpsertRow> = tracks
            .iter()
            .enumerate()
            .map(|(i, title)| TrackUpsertRow {
                title: (*title).into(),
                album_id,
                artist_id,
                track_number: Some(i as i32 + 1),
                disc_number: Some(1),
                duration_ms: Some(200000),
                source_id: format!("{}-t{}", album_key, i),
                codec: Some("mp3".into()),
                part_key: None,
                stream_id: None,
                user_rating: None,
                bitrate: Some(320),
                track_artist: None,
                updated_at: None,
                file_size_bytes: None,
                rating_count: None,
            })
            .collect();
        if !rows.is_empty() {
            db.batch_upsert_tracks(&rows).unwrap();
        }
        (artist_id, album_id)
    }

    // ---- Artist section ----

    #[test]
    fn test_free_text_exact_artist_returns_all_sections_in_order() {
        let (_db, engine) = setup();
        let resp = run(&engine, "radiohead");

        let artists = artists_of(&resp);
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].name, "Radiohead");
        assert_eq!(artists[0].album_count, 2);
        assert_eq!(artists[0].score, 0.0);

        // Albums arrive via the artist-name field and artist seeding.
        let titles = album_titles(&resp);
        assert!(titles.contains(&"OK Computer".to_string()));
        assert!(titles.contains(&"Kid A".to_string()));
        assert!(!titles.contains(&"Reign in Blood".to_string()));

        // Exact artist match fills the track section with their tracks.
        let tracks = track_titles(&resp);
        assert!(tracks.contains(&"Karma Police".to_string()));

        assert_eq!(section_kinds(&resp), vec!["artists", "albums", "tracks"]);
    }

    #[test]
    fn test_artist_prefix_match() {
        let (_db, engine) = setup();
        let resp = run(&engine, "radioh");
        let artists = artists_of(&resp);
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].name, "Radiohead");
        assert_eq!(artists[0].score, SCORE_PREFIX);
        // Prefix (non-exact) artist match must NOT fill tracks.
        assert!(tracks_of(&resp).is_empty(), "no track fill for prefix match");
        // But their albums are still seeded.
        assert!(album_titles(&resp).contains(&"Kid A".to_string()));
    }

    #[test]
    fn test_artist_typo_fuzzy_match_seeds_albums() {
        let (_db, engine) = setup();
        let resp = run(&engine, "radiohed");
        let artists = artists_of(&resp);
        assert!(
            artists.iter().any(|a| a.name == "Radiohead"),
            "typo should fuzzy-match the artist"
        );
        let titles = album_titles(&resp);
        assert!(
            titles.contains(&"OK Computer".to_string()) && titles.contains(&"Kid A".to_string()),
            "fuzzy-matched artist's albums should seed the album section, got {:?}",
            titles
        );
        // Artists must outrank the seeded albums.
        assert_eq!(section_kinds(&resp)[0], "artists");
    }

    #[test]
    fn test_artist_diacritic_insensitive() {
        let (db, engine) = setup();
        seed_artist_album(&db, "Asphodèle", "artist-asph", "Jours Pâles", "album-jp", &[]);

        let resp = run(&engine, "asphodele");
        let artists = artists_of(&resp);
        assert_eq!(artists.len(), 1);
        assert_eq!(artists[0].name, "Asphodèle");
        assert_eq!(artists[0].score, 0.0, "folded compare should be exact");

        let resp2 = run(&engine, "jours pales");
        assert!(album_titles(&resp2).contains(&"Jours Pâles".to_string()));
    }

    #[test]
    fn test_multi_word_artist_contains_match() {
        let (db, engine) = setup();
        seed_artist_album(
            &db,
            "The Mountain Goats",
            "artist-tmg",
            "Tallahassee",
            "album-tal",
            &["No Children"],
        );
        // Without the leading article the name still matches (contains).
        let resp = run(&engine, "mountain goats");
        assert!(artists_of(&resp).iter().any(|a| a.name == "The Mountain Goats"));
        // Full name is an exact match and fills tracks.
        let resp2 = run(&engine, "the mountain goats");
        assert_eq!(artists_of(&resp2)[0].score, 0.0);
        assert!(track_titles(&resp2).contains(&"No Children".to_string()));
    }

    // ---- Albums ----

    #[test]
    fn test_album_title_prefix_beats_artist_field() {
        let (_db, engine) = setup();
        let resp = run(&engine, "kid");
        let albums = albums_of(&resp);
        assert!(!albums.is_empty());
        assert_eq!(albums[0].title, "Kid A");
    }

    #[test]
    fn test_album_quality_badge() {
        let (_db, engine) = setup();
        let resp = run(&engine, "kid a");
        let albums = albums_of(&resp);
        assert_eq!(albums[0].quality.as_deref(), Some("FLAC"));
    }

    #[test]
    fn test_album_quality_badge_lossy_includes_bitrate() {
        let (db, engine) = setup();
        seed_artist_album(&db, "Joshua Radin", "artist-jr", "Simple Times", "album-st", &["Vegetable Car"]);
        let resp = run(&engine, "simple times");
        let albums = albums_of(&resp);
        assert_eq!(albums[0].quality.as_deref(), Some("MP3 320"));
    }

    #[test]
    fn test_free_text_cross_field_artist_plus_album_title() {
        let (_db, engine) = setup();
        let resp = run(&engine, "radiohead ok");
        assert!(
            album_titles(&resp).contains(&"OK Computer".to_string()),
            "token-AND album search should find OK Computer"
        );
    }

    #[test]
    fn test_fuzzy_album_fallback_finds_typo() {
        let (db, engine) = setup();
        seed_artist_album(
            &db,
            "Bomb the Music Industry!",
            "artist-btmi",
            "Adults!!!",
            "album-adults",
            &[],
        );
        let resp = run(&engine, "adluts");
        assert!(
            album_titles(&resp).contains(&"Adults!!!".to_string()),
            "Fuzzy album fallback should find 'Adults!!!' for typo 'adluts'"
        );
    }

    // ---- Tracks ----

    #[test]
    fn test_exact_track_title_puts_tracks_section_first() {
        let (_db, engine) = setup();
        let resp = run(&engine, "karma police");
        assert_eq!(section_kinds(&resp)[0], "tracks");
        assert_eq!(tracks_of(&resp)[0].title, "Karma Police");
        assert_eq!(tracks_of(&resp)[0].score, 0.0);
    }

    #[test]
    fn test_free_text_cross_field_artist_plus_track_title() {
        // "radiohead AND karma" merges to FreeText("radiohead karma"). FTS5
        // alone can't match (tracks_fts indexes only title, and no title
        // contains both tokens). The cross-field search resolves it via
        // artist=Radiohead + track title=Karma Police.
        let (_db, engine) = setup();
        let resp = run(&engine, "radiohead AND karma");
        assert!(
            track_titles(&resp).contains(&"Karma Police".to_string()),
            "cross-field search should find Karma Police by Radiohead"
        );
    }

    #[test]
    fn test_track_search_fuzzy_fallback() {
        let (_db, engine) = setup();
        let resp = run(&engine, "!paranoyd");
        assert!(
            track_titles(&resp).contains(&"Paranoid Android".to_string()),
            "Fuzzy should find 'Paranoid Android' for typo 'paranoyd'"
        );
    }

    #[test]
    fn test_fuzzy_rejects_short_false_positives() {
        let (db, engine) = setup();
        seed_artist_album(
            &db,
            "Bomb the Music Industry!",
            "artist-btmi",
            "Adults!!!",
            "album-adults",
            &["Fault", "Salt", "Souls"],
        );
        let resp = run(&engine, "adults");
        let tracks = track_titles(&resp);
        for junk in ["Fault", "Salt", "Souls"] {
            assert!(
                !tracks.contains(&junk.to_string()),
                "Short false-positive '{}' should not appear for query 'adults'",
                junk
            );
        }
        assert!(album_titles(&resp).contains(&"Adults!!!".to_string()));
    }

    #[test]
    fn test_fuzzy_cross_field_track_match() {
        let (db, engine) = setup();
        seed_artist_album(
            &db,
            "Bomb the Music Industry!",
            "artist-btmi",
            "Adults!!!",
            "album-adults",
            &["It's Just Brains"],
        );
        let resp = run(&engine, "!adluts");
        assert!(
            tracks_of(&resp).iter().any(|t| t.album_title == "Adults!!!"),
            "Fuzzy cross-field should find tracks from album 'Adults!!!' for typo 'adluts'"
        );
    }

    #[test]
    fn test_track_search_operator_returns_tracks_only() {
        let (_db, engine) = setup();
        let resp = run(&engine, "!paranoid");
        assert_eq!(section_kinds(&resp), vec!["tracks"]);
        assert_eq!(tracks_of(&resp)[0].title, "Paranoid Android");
    }

    // ---- Genres ----

    #[test]
    fn test_genre_section_matches_library_genres() {
        let (_db, engine) = setup();
        let resp = run(&engine, "rock");
        let genres = genres_of(&resp);
        assert_eq!(genres.len(), 1);
        assert_eq!(genres[0].name, "Rock");
        assert_eq!(genres[0].album_count, 2);
        assert_eq!(genres[0].score, SCORE_EXACT + GENRE_SECTION_PENALTY);
    }

    #[test]
    fn test_genre_section_ranks_behind_equal_artist_match() {
        let (db, engine) = setup();
        seed_artist_album(&db, "Metallica", "artist-met", "Ride the Lightning", "album-rtl", &[]);
        let resp = run(&engine, "metal");
        let kinds = section_kinds(&resp);
        let artist_pos = kinds.iter().position(|k| *k == "artists").unwrap();
        let genre_pos = kinds.iter().position(|k| *k == "genres").unwrap();
        assert!(
            artist_pos < genre_pos,
            "prefix artist match should outrank exact genre (kinds: {:?})",
            kinds
        );
        assert!(genres_of(&resp).iter().any(|g| g.name == "Metal"));
    }

    #[test]
    fn test_genre_section_no_fuzzy() {
        let (_db, engine) = setup();
        let resp = run(&engine, "rockk");
        // "rockk" doesn't literally match "Rock"; fuzzy is disabled for
        // genres so the section must be absent.
        assert!(genres_of(&resp).is_empty());
    }

    // ---- Operators & constraints ----

    #[test]
    fn test_artist_operator_returns_artists_and_their_albums() {
        let (_db, engine) = setup();
        let resp = run(&engine, "@slayer");
        assert_eq!(artists_of(&resp).len(), 1);
        assert_eq!(artists_of(&resp)[0].name, "Slayer");
        let titles = album_titles(&resp);
        assert_eq!(titles, vec!["Reign in Blood".to_string()]);
        // `@` must not produce genre results.
        assert!(genres_of(&resp).is_empty());
    }

    #[test]
    fn test_album_title_operator() {
        let (_db, engine) = setup();
        let resp = run(&engine, "%ok computer");
        assert_eq!(section_kinds(&resp), vec!["albums"]);
        assert_eq!(albums_of(&resp)[0].title, "OK Computer");
    }

    #[test]
    fn test_genre_filter_returns_albums() {
        let (_db, engine) = setup();
        let resp = run(&engine, "/rock");
        assert_eq!(section_kinds(&resp), vec!["albums"]);
        let titles = album_titles(&resp);
        assert!(titles.contains(&"OK Computer".to_string()));
        assert!(titles.contains(&"Kid A".to_string()));
        assert!(!titles.contains(&"Reign in Blood".to_string()));
    }

    #[test]
    fn test_genre_filter_expands_hierarchy() {
        let db = Arc::new(CacheDatabase::open_in_memory().unwrap());
        seed_test_data(&db);

        // Test expander: "Electronic" is a child of "Rock".
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
        let resp = run(&engine, "/rock");
        let titles = album_titles(&resp);
        assert!(titles.contains(&"OK Computer".to_string()), "Should include Rock-tagged album");
        assert!(
            titles.contains(&"Kid A".to_string()),
            "Should include Electronic-tagged album (child of Rock)"
        );
        assert!(!titles.contains(&"Reign in Blood".to_string()));
    }

    #[test]
    fn test_genre_filter_bad_fuzzy_match_falls_back_to_raw_name() {
        // Simulates a genre the user has tagged in Plex (e.g. "Blackgaze")
        // that isn't in the curated tree. The mapper's fuzzy match
        // returns a nearby tree family (Black Metal) that doesn't include
        // the original name. The engine must fall back to a direct DB
        // lookup on the raw name so the user's actual tagged albums
        // surface instead of the wrong-family neighbour.
        let db = Arc::new(CacheDatabase::open_in_memory().unwrap());
        seed_test_data(&db);

        let artist_map = db
            .batch_upsert_artists(&[("Deafheaven".into(), None, "artist-3".into(), None, None, None, None)])
            .unwrap();
        let deafheaven_id = *artist_map.get("artist-3").unwrap();
        let album_map = db
            .batch_upsert_albums(&[AlbumUpsertRow {
                title: "Sunbather".into(),
                artist_id: deafheaven_id,
                year: Some(2013),
                source_id: "album-4".into(),
                art_url: None,
                updated_at: None,
                added_at: None,
                last_viewed_at: None,
                first_genre: None,
                first_collection: None,
                view_count: None,
            }])
            .unwrap();
        let sunbather_id = *album_map.get("album-4").unwrap();
        let blackgaze_id = db.upsert_genre("Blackgaze").unwrap();
        db.set_album_genres(sunbather_id, &[blackgaze_id]).unwrap();

        struct BadMatchExpander;
        impl GenreExpander for BadMatchExpander {
            fn expand_genre(&self, name: &str) -> Option<HashSet<String>> {
                if name.eq_ignore_ascii_case("blackgaze") {
                    let mut set = HashSet::new();
                    set.insert("Metal".to_string());
                    Some(set)
                } else {
                    None
                }
            }
        }

        let engine = SearchEngine::new(db, Some(Arc::new(BadMatchExpander)));
        let resp = run(&engine, "/blackgaze");
        let titles = album_titles(&resp);
        assert!(
            titles.contains(&"Sunbather".to_string()),
            "Blackgaze-tagged album must surface despite the mapper fuzzy-matching to Metal"
        );
        assert!(
            !titles.contains(&"Reign in Blood".to_string()),
            "The fuzzy-matched Metal family must not pollute results for an exact-tagged genre"
        );
    }

    #[test]
    fn test_year_range_filter() {
        let (_db, engine) = setup();
        let resp = run(&engine, "year:>1999");
        assert_eq!(album_titles(&resp), vec!["Kid A".to_string()]);
        // Constraint queries never emit artist/genre sections.
        assert!(artists_of(&resp).is_empty());
        assert!(genres_of(&resp).is_empty());
    }

    #[test]
    fn test_combined_filters() {
        let (_db, engine) = setup();
        let resp = run(&engine, "/rock AND year:>1999");
        assert_eq!(album_titles(&resp), vec!["Kid A".to_string()]);
    }

    #[test]
    fn test_collection_filter_returns_matching_albums() {
        let (db, engine) = setup();
        let ok_id = db.album_id("album-1").unwrap().unwrap();
        let kid_a_id = db.album_id("album-3").unwrap().unwrap();

        let sleep_id = db.upsert_collection("Sleep").unwrap();
        db.link_album_collection(ok_id, sleep_id).unwrap();
        db.link_album_collection(kid_a_id, sleep_id).unwrap();

        let resp = run(&engine, "col:Sleep");
        let titles = album_titles(&resp);
        assert!(titles.contains(&"OK Computer".to_string()));
        assert!(titles.contains(&"Kid A".to_string()));
        assert!(!titles.contains(&"Reign in Blood".to_string()));
    }

    #[test]
    fn test_collection_filter_combined_with_genre() {
        let (db, engine) = setup();
        let ok_id = db.album_id("album-1").unwrap().unwrap();
        let reign_id = db.album_id("album-2").unwrap().unwrap();

        let sleep_id = db.upsert_collection("Sleep").unwrap();
        db.link_album_collection(ok_id, sleep_id).unwrap();
        db.link_album_collection(reign_id, sleep_id).unwrap();

        let resp = run(&engine, "col:Sleep AND /rock");
        assert_eq!(album_titles(&resp), vec!["OK Computer".to_string()]);
    }

    #[test]
    fn test_collection_filter_no_match_returns_empty() {
        let (_db, engine) = setup();
        let resp = run(&engine, "col:Nonexistent");
        assert!(resp.sections.is_empty());
    }

    // ---- General ----

    #[test]
    fn test_empty_query() {
        let (_db, engine) = setup();
        let resp = run(&engine, "");
        assert!(resp.sections.is_empty());
    }

    #[test]
    fn test_gibberish_returns_empty() {
        let (_db, engine) = setup();
        let resp = run(&engine, "zzzznonexistent");
        assert!(resp.sections.is_empty(), "No results for gibberish query");
    }

    #[test]
    fn test_sections_capped_at_limit() {
        let (db, engine) = setup();
        for i in 0..30 {
            seed_artist_album(
                &db,
                &format!("Test Band {}", i),
                &format!("artist-tb{}", i),
                &format!("Test Album {}", i),
                &format!("album-tb{}", i),
                &[],
            );
        }
        let resp = engine
            .search_sectioned(&QueryParser::parse("test"), 5)
            .unwrap();
        assert!(artists_of(&resp).len() <= 5);
        assert!(albums_of(&resp).len() <= 5);
    }
}
