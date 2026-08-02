use std::collections::HashSet;

use rusqlite::params;

use crate::models::{RangeOp, Track};
use crate::util::{escape_fts5, escape_like};

use super::db::{
    AlbumSearchRow, ArtistSearchRow, CacheDatabase, CacheError, GenreSearchRow, TrackSearchRow,
};

/// (codec, bitrate) per album source id, for quality badges.
pub type AlbumCodecInfo = std::collections::HashMap<String, (Option<String>, Option<i64>)>;

fn map_album_search_row(row: &rusqlite::Row) -> rusqlite::Result<AlbumSearchRow> {
    let rating: Option<f64> = row.get(5)?;
    Ok(AlbumSearchRow {
        album_source_id: row.get(0)?,
        album_title: row.get(1)?,
        artist_name: row.get(2)?,
        year: row.get(3)?,
        art_url: row.get(4)?,
        rating,
        is_favourite: rating.map(|r| r >= 10.0).unwrap_or(false),
    })
}

fn map_track_search_row(row: &rusqlite::Row) -> rusqlite::Result<TrackSearchRow> {
    let user_rating: Option<f64> = row.get(8)?;
    Ok(TrackSearchRow {
        id: row.get(0)?,
        track_source_id: row.get(1)?,
        track_title: row.get(2)?,
        artist_name: row.get(3)?,
        album_title: row.get(4)?,
        album_source_id: row.get(5)?,
        art_url: row.get(6)?,
        track_artist: row.get(7)?,
        user_rating,
        is_favourite: user_rating.map(|r| r >= 10.0).unwrap_or(false),
    })
}

/// Build an optional ` AND a.id IN (?, ?, ...)` clause and collect its IDs.
/// Returns `(sql_fragment, id_vec)`. When `album_ids` is `None`, both are empty.
fn build_id_filter(album_ids: Option<&HashSet<i64>>) -> (String, Vec<i64>) {
    match album_ids {
        Some(ids) if !ids.is_empty() => {
            let placeholders = (0..ids.len()).map(|_| "?").collect::<Vec<_>>().join(",");
            let id_vec: Vec<i64> = ids.iter().copied().collect();
            (format!(" AND a.id IN ({})", placeholders), id_vec)
        }
        _ => (String::new(), Vec::new()),
    }
}

/// Variant of `build_id_filter` that binds against `t.albumId` for track queries.
fn build_track_id_filter(album_ids: Option<&HashSet<i64>>) -> (String, Vec<i64>) {
    match album_ids {
        Some(ids) if !ids.is_empty() => {
            let placeholders = (0..ids.len()).map(|_| "?").collect::<Vec<_>>().join(",");
            let id_vec: Vec<i64> = ids.iter().copied().collect();
            (format!(" AND t.albumId IN ({})", placeholders), id_vec)
        }
        _ => (String::new(), Vec::new()),
    }
}

impl CacheDatabase {
    /// FTS5 prefix search on track titles.
    pub fn search_tracks_fts(&self, query: &str) -> Result<Vec<Track>, CacheError> {
        let conn = self.conn.lock();
        let escaped = escape_fts5(query);
        let fts_query: String = escaped
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| format!("\"{}\"*", t))
            .collect::<Vec<_>>()
            .join(" ");
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = conn.prepare(
            "SELECT t.sourceId, t.title, ar.name, t.trackArtist,
                    al.title, al.sourceId, t.trackNumber, t.durationMs,
                    t.codec, t.partKey, al.artUrl, t.userRating, t.bitrate, t.discNumber,
                    t.fileSizeBytes, t.ratingCount
             FROM tracks_fts fts
             JOIN tracks t ON t.id = fts.rowid
             JOIN albums al ON al.id = t.albumId
             JOIN artists ar ON ar.id = t.artistId
             WHERE tracks_fts MATCH ?1
             ORDER BY rank",
        )?;
        let tracks = stmt
            .query_map(params![fts_query], Self::map_track_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks)
    }

    /// Search albums by title using LIKE contains.
    pub fn search_albums_by_title(
        &self,
        query: &str,
    ) -> Result<Vec<crate::models::Album>, CacheError> {
        let conn = self.conn.lock();
        let pattern = format!("%{}%", escape_like(query));
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl,
                    a.rating, a.studio, a.addedAt, a.lastViewedAt,
                    a.viewCount, a.format, ar.country
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE a.title LIKE ?1 ESCAPE '\\'
             ORDER BY a.title COLLATE NOCASE",
        )?;
        let mut albums = Self::map_album_rows(&mut stmt, params![pattern], &conn)?;
        drop(stmt);
        drop(conn);
        self.populate_album_genres(&mut albums)?;
        self.populate_album_collections(&mut albums)?;
        self.populate_album_favourite_tracks(&mut albums)?;
        Ok(albums)
    }

    /// Album IDs where year matches the given range op.
    pub fn albums_by_year_range(
        &self,
        op: RangeOp,
        value: i32,
    ) -> Result<HashSet<i64>, CacheError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT id FROM albums WHERE year IS NOT NULL AND year {} ?1",
            op.sql_literal()
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![value], |row| row.get::<_, i64>(0))?;
        let mut set = HashSet::new();
        for row in rows {
            set.insert(row?);
        }
        Ok(set)
    }

    /// Album IDs where rating matches the given range op.
    pub fn albums_by_rating_range(
        &self,
        op: RangeOp,
        value: f64,
    ) -> Result<HashSet<i64>, CacheError> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT id FROM albums WHERE rating IS NOT NULL AND rating {} ?1",
            op.sql_literal()
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![value], |row| row.get::<_, i64>(0))?;
        let mut set = HashSet::new();
        for row in rows {
            set.insert(row?);
        }
        Ok(set)
    }

    /// Search albums by title (LIKE) with optional album-ID filter.
    pub fn search_albums_by_title_filtered(
        &self,
        query: &str,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<AlbumSearchRow>, CacheError> {
        if matches!(album_ids, Some(ids) if ids.is_empty()) {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let pattern = format!("%{}%", escape_like(query));
        let (id_filter, id_vec) = build_id_filter(album_ids);
        let sql = format!(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE a.title LIKE ?1 ESCAPE '\\'{}
             ORDER BY a.title COLLATE NOCASE
             LIMIT ?2",
            id_filter
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.push(Box::new(pattern));
        for id in &id_vec {
            all_params.push(Box::new(*id));
        }
        all_params.push(Box::new(limit as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), map_album_search_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Search albums by artist name (LIKE) with optional album-ID filter.
    pub fn search_albums_by_artist(
        &self,
        query: &str,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<AlbumSearchRow>, CacheError> {
        if matches!(album_ids, Some(ids) if ids.is_empty()) {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let pattern = format!("%{}%", escape_like(query));
        let (id_filter, id_vec) = build_id_filter(album_ids);
        let sql = format!(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE ar.name LIKE ?1 ESCAPE '\\'{}
             ORDER BY ar.name COLLATE NOCASE, a.year
             LIMIT ?2",
            id_filter
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.push(Box::new(pattern));
        for id in &id_vec {
            all_params.push(Box::new(*id));
        }
        all_params.push(Box::new(limit as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), map_album_search_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Search albums by artist OR title (LIKE) with optional album-ID filter.
    ///
    /// Tokenises on whitespace; each token must match either the artist name
    /// or the album title, and tokens are ANDed. Multi-token queries like
    /// `"radiohead ok computer"` match across fields — `"radiohead"` against
    /// the artist, `"ok"`/`"computer"` against the title.
    pub fn search_albums_by_artist_or_title(
        &self,
        query: &str,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<AlbumSearchRow>, CacheError> {
        if matches!(album_ids, Some(ids) if ids.is_empty()) {
            return Ok(Vec::new());
        }
        let tokens: Vec<String> = query
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| format!("%{}%", escape_like(t)))
            .collect();
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let (id_filter, id_vec) = build_id_filter(album_ids);
        // Each token binds twice — once per column it could match (artist
        // name or album title).
        let token_clauses: Vec<&str> = tokens
            .iter()
            .map(|_| "(ar.name LIKE ? ESCAPE '\\' OR a.title LIKE ? ESCAPE '\\')")
            .collect();
        let where_tokens = token_clauses.join(" AND ");
        let sql = format!(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE {}{}
             ORDER BY a.title COLLATE NOCASE
             LIMIT ?",
            where_tokens, id_filter
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for pattern in &tokens {
            all_params.push(Box::new(pattern.clone()));
            all_params.push(Box::new(pattern.clone()));
        }
        for id in &id_vec {
            all_params.push(Box::new(*id));
        }
        all_params.push(Box::new(limit as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), map_album_search_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Cross-field tokenised track search. Each whitespace-separated token
    /// must match at least one of (track title, album title, artist name)
    /// and tokens are ANDed. Fills the gap FTS5 cannot cover: `tracks_fts`
    /// indexes only the track title, so a query like `"radiohead karma"`
    /// returns no FTS5 hits. This joins on albums and artists so tokens may
    /// span fields.
    pub fn search_tracks_by_tokens_cross_field(
        &self,
        query: &str,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<TrackSearchRow>, CacheError> {
        if matches!(album_ids, Some(ids) if ids.is_empty()) {
            return Ok(Vec::new());
        }
        let tokens: Vec<String> = query
            .split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| format!("%{}%", escape_like(t)))
            .collect();
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let (id_filter, id_vec) = build_track_id_filter(album_ids);
        // Each token binds three times — one per column (track title,
        // album title, artist name).
        let token_clauses: Vec<&str> = tokens
            .iter()
            .map(|_| "(t.title LIKE ? ESCAPE '\\' OR al.title LIKE ? ESCAPE '\\' OR ar.name LIKE ? ESCAPE '\\')")
            .collect();
        let where_tokens = token_clauses.join(" AND ");
        let sql = format!(
            "SELECT t.id, t.sourceId, t.title, ar.name, al.title, al.sourceId, al.artUrl, t.trackArtist, t.userRating
             FROM tracks t
             JOIN albums al ON al.id = t.albumId
             JOIN artists ar ON ar.id = t.artistId
             WHERE {}{}
             ORDER BY t.title COLLATE NOCASE
             LIMIT ?",
            where_tokens, id_filter
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for pattern in &tokens {
            all_params.push(Box::new(pattern.clone()));
            all_params.push(Box::new(pattern.clone()));
            all_params.push(Box::new(pattern.clone()));
        }
        for id in &id_vec {
            all_params.push(Box::new(*id));
        }
        all_params.push(Box::new(limit as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), map_track_search_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Search albums by ID filter only (for genre/year/fav filter-only queries) in random order.
    pub fn search_albums_filtered(
        &self,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<AlbumSearchRow>, CacheError> {
        let ids = match album_ids {
            Some(ids) if !ids.is_empty() => ids,
            _ => return Ok(Vec::new()),
        };
        let conn = self.conn.lock();
        let placeholders = (0..ids.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE a.id IN ({})
             ORDER BY RANDOM()
             LIMIT ?",
            placeholders
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for id in ids {
            all_params.push(Box::new(*id));
        }
        all_params.push(Box::new(limit as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), map_album_search_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// FTS5 enriched track search with album/artist joins.
    pub fn search_tracks_enriched(
        &self,
        fts_query: &str,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<TrackSearchRow>, CacheError> {
        if matches!(album_ids, Some(ids) if ids.is_empty()) {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let (id_filter, id_vec) = build_track_id_filter(album_ids);
        let sql = format!(
            "SELECT t.id, t.sourceId, t.title, ar.name, al.title, al.sourceId, al.artUrl, t.trackArtist, t.userRating
             FROM tracks_fts fts
             JOIN tracks t ON t.id = fts.rowid
             JOIN albums al ON al.id = t.albumId
             JOIN artists ar ON ar.id = t.artistId
             WHERE tracks_fts MATCH ?1{}
             ORDER BY rank
             LIMIT ?2",
            id_filter
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        all_params.push(Box::new(fts_query.to_string()));
        for id in &id_vec {
            all_params.push(Box::new(*id));
        }
        all_params.push(Box::new(limit as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), map_track_search_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Tracks as fuzzy search candidates.
    pub fn search_candidates(
        &self,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<TrackSearchRow>, CacheError> {
        if matches!(album_ids, Some(ids) if ids.is_empty()) {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let (id_filter, id_vec) = build_track_id_filter(album_ids);
        // Strip the leading " AND" so the fragment becomes a standalone WHERE.
        let where_clause = if id_filter.is_empty() {
            String::new()
        } else {
            format!(" WHERE{}", &id_filter[4..])
        };
        let sql = format!(
            "SELECT t.id, t.sourceId, t.title, ar.name, al.title, al.sourceId, al.artUrl, t.trackArtist, t.userRating
             FROM tracks t
             JOIN albums al ON al.id = t.albumId
             JOIN artists ar ON ar.id = t.artistId{}
             LIMIT ?",
            where_clause
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for id in &id_vec {
            all_params.push(Box::new(*id));
        }
        all_params.push(Box::new(limit as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), map_track_search_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Albums as fuzzy search candidates.
    pub fn search_album_candidates(
        &self,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<AlbumSearchRow>, CacheError> {
        if matches!(album_ids, Some(ids) if ids.is_empty()) {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
        let (id_filter, id_vec) = build_id_filter(album_ids);
        let where_clause = if id_filter.is_empty() {
            String::new()
        } else {
            format!(" WHERE{}", &id_filter[4..])
        };
        let sql = format!(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId{}
             LIMIT ?",
            where_clause
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for id in &id_vec {
            all_params.push(Box::new(*id));
        }
        all_params.push(Box::new(limit as i64));
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), map_album_search_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Internal album IDs matching album title (LIKE contains). Unlimited.
    pub fn album_ids_by_title(
        &self,
        query: &str,
        constrain_to: Option<&HashSet<i64>>,
    ) -> Result<HashSet<i64>, CacheError> {
        if matches!(constrain_to, Some(ids) if ids.is_empty()) {
            return Ok(HashSet::new());
        }
        let conn = self.conn.lock();
        let pattern = format!("%{}%", escape_like(query));
        let (id_filter, id_vec) = build_id_filter(constrain_to);
        let sql = format!(
            "SELECT DISTINCT a.id FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE a.title LIKE ?1 ESCAPE '\\'{}",
            id_filter
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(pattern)];
        for id in &id_vec {
            all_params.push(Box::new(*id));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, i64>(0))?;
        let mut set = HashSet::new();
        for row in rows {
            set.insert(row?);
        }
        Ok(set)
    }

    /// Internal album IDs matching artist name (LIKE contains). Unlimited.
    pub fn album_ids_by_artist(
        &self,
        query: &str,
        constrain_to: Option<&HashSet<i64>>,
    ) -> Result<HashSet<i64>, CacheError> {
        if matches!(constrain_to, Some(ids) if ids.is_empty()) {
            return Ok(HashSet::new());
        }
        let conn = self.conn.lock();
        let pattern = format!("%{}%", escape_like(query));
        let (id_filter, id_vec) = build_id_filter(constrain_to);
        let sql = format!(
            "SELECT DISTINCT a.id FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE ar.name LIKE ?1 ESCAPE '\\'{}",
            id_filter
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(pattern)];
        for id in &id_vec {
            all_params.push(Box::new(*id));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, i64>(0))?;
        let mut set = HashSet::new();
        for row in rows {
            set.insert(row?);
        }
        Ok(set)
    }

    /// Internal album IDs matching artist name OR album title (LIKE contains). Unlimited.
    pub fn album_ids_by_artist_or_title(
        &self,
        query: &str,
        constrain_to: Option<&HashSet<i64>>,
    ) -> Result<HashSet<i64>, CacheError> {
        if matches!(constrain_to, Some(ids) if ids.is_empty()) {
            return Ok(HashSet::new());
        }
        let conn = self.conn.lock();
        let pattern = format!("%{}%", escape_like(query));
        let (id_filter, id_vec) = build_id_filter(constrain_to);
        let sql = format!(
            "SELECT DISTINCT a.id FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE (ar.name LIKE ?1 ESCAPE '\\' OR a.title LIKE ?1 ESCAPE '\\'){}",
            id_filter
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(pattern)];
        for id in &id_vec {
            all_params.push(Box::new(*id));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, i64>(0))?;
        let mut set = HashSet::new();
        for row in rows {
            set.insert(row?);
        }
        Ok(set)
    }

    /// All artists that own at least one album, with album counts.
    /// Candidate set for in-memory artist matching — artist tables are
    /// small enough (thousands) that scoring in Rust beats trying to make
    /// SQL LIKE diacritic-insensitive.
    pub fn artist_search_candidates(&self) -> Result<Vec<ArtistSearchRow>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT ar.id, ar.sourceId, ar.name, ar.artUrl, COUNT(a.id) AS albumCount
             FROM artists ar
             JOIN albums a ON a.artistId = ar.id
             GROUP BY ar.id
             ORDER BY ar.name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ArtistSearchRow {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    name: row.get(2)?,
                    art_url: row.get(3)?,
                    album_count: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// All genres linked to at least one album, with album counts.
    /// Candidate set for in-memory genre matching.
    pub fn genre_search_candidates(&self) -> Result<Vec<GenreSearchRow>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT g.name, COUNT(DISTINCT ag.albumId) AS albumCount
             FROM genres g
             JOIN album_genres ag ON ag.genreId = g.id
             GROUP BY g.id
             ORDER BY g.name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(GenreSearchRow {
                    name: row.get(0)?,
                    album_count: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Albums owned by a single artist (internal id), best-rated first.
    /// Used to seed album results from a matched artist — the artist may
    /// have been fuzzy-matched, so a name LIKE would miss.
    pub fn albums_for_artist_id(
        &self,
        artist_id: i64,
        limit: usize,
    ) -> Result<Vec<AlbumSearchRow>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE a.artistId = ?1
             ORDER BY a.rating DESC NULLS LAST, a.year DESC NULLS LAST, a.title COLLATE NOCASE
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![artist_id, limit as i64], map_album_search_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Tracks by a single artist (internal id), best-rated first. Used to
    /// fill the track section when a strong artist match has no direct
    /// track-title matches to show.
    pub fn tracks_for_artist_id(
        &self,
        artist_id: i64,
        limit: usize,
    ) -> Result<Vec<TrackSearchRow>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT t.id, t.sourceId, t.title, ar.name, al.title, al.sourceId, al.artUrl, t.trackArtist, t.userRating
             FROM tracks t
             JOIN albums al ON al.id = t.albumId
             JOIN artists ar ON ar.id = t.artistId
             WHERE t.artistId = ?1
             ORDER BY t.userRating DESC NULLS LAST, t.ratingCount DESC NULLS LAST, t.id
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![artist_id, limit as i64], map_track_search_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Representative (codec, bitrate) per album, for quality badges on
    /// album search rows. One row per album from its first track; albums
    /// mixing codecs get whichever track sorts first, which is fine for a
    /// display hint.
    pub fn album_codec_info(&self, album_source_ids: &[String]) -> Result<AlbumCodecInfo, CacheError> {
        if album_source_ids.is_empty() {
            return Ok(AlbumCodecInfo::new());
        }
        let conn = self.conn.lock();
        let placeholders = (0..album_source_ids.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT al.sourceId, t.codec, t.bitrate
             FROM albums al
             JOIN tracks t ON t.albumId = al.id
             WHERE al.sourceId IN ({})
             GROUP BY al.id",
            placeholders
        );
        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = album_source_ids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (row.get::<_, Option<String>>(1)?, row.get::<_, Option<i64>>(2)?),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().collect())
    }
}
