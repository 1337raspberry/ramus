use std::collections::HashSet;
use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{params, Connection};

use crate::models::{Album, Track};

// Re-export upsert types for external consumers
pub use super::upsert::{AlbumUpsertRow, ArtistRow, ArtistUpsertRow, TrackUpsertRow};

// --- Errors ---

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// --- Helper types ---

#[derive(Debug, Clone)]
pub struct CachedItemInfo {
    pub id: i64,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CachedAlbumInfo {
    pub id: i64,
    pub updated_at: Option<i64>,
    pub first_genre: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub artist_count: i64,
    pub album_count: i64,
    pub track_count: i64,
    pub genre_count: i64,
}

// --- Search result row types ---

/// Album row returned by search queries.
#[derive(Debug, Clone)]
pub struct AlbumSearchRow {
    pub album_source_id: String,
    pub album_title: String,
    pub artist_name: String,
    pub year: Option<i32>,
    pub art_url: Option<String>,
    pub is_favourite: bool,
}

/// Track row returned by enriched search queries.
#[derive(Debug, Clone)]
pub struct TrackSearchRow {
    pub id: i64,
    pub track_source_id: String,
    pub track_title: String,
    pub artist_name: String,
    pub album_title: String,
    pub album_source_id: String,
    pub art_url: Option<String>,
    pub track_artist: Option<String>,
    pub is_favourite: bool,
}

// --- CacheDatabase ---

pub struct CacheDatabase {
    pub(super) conn: Mutex<Connection>,
}

impl CacheDatabase {
    /// Open (or create) a database at the given path with WAL mode and migrations.
    pub fn open(path: &Path) -> Result<Self, CacheError> {
        let conn = Connection::open(path)?;
        Self::configure_and_migrate(conn)
    }

    /// Open an in-memory database (for testing).
    pub fn open_in_memory() -> Result<Self, CacheError> {
        let conn = Connection::open_in_memory()?;
        Self::configure_and_migrate(conn)
    }

    fn configure_and_migrate(conn: Connection) -> Result<Self, CacheError> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?;
        Self::run_migration(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn run_migration(conn: &Connection) -> Result<(), CacheError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS artists (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                sortName TEXT,
                sourceId TEXT NOT NULL UNIQUE,
                artUrl TEXT,
                summary TEXT,
                updatedAt INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_artists_name ON artists(name);

            CREATE TABLE IF NOT EXISTS albums (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                artistId INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
                year INTEGER,
                sourceId TEXT NOT NULL UNIQUE,
                artUrl TEXT,
                updatedAt INTEGER,
                rating DOUBLE,
                studio TEXT,
                ultraBlurColors TEXT,
                vibrantPalette TEXT,
                addedAt INTEGER,
                lastViewedAt INTEGER,
                firstGenre TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_albums_title ON albums(title);
            CREATE INDEX IF NOT EXISTS idx_albums_artistId ON albums(artistId);
            CREATE INDEX IF NOT EXISTS idx_albums_rating ON albums(rating);
            CREATE INDEX IF NOT EXISTS idx_albums_year ON albums(year);

            CREATE TABLE IF NOT EXISTS tracks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                albumId INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
                artistId INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
                trackNumber INTEGER,
                discNumber INTEGER,
                durationMs INTEGER,
                sourceId TEXT NOT NULL UNIQUE,
                codec TEXT,
                partKey TEXT,
                updatedAt INTEGER,
                streamId INTEGER,
                userRating DOUBLE,
                bitrate INTEGER,
                trackArtist TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_tracks_albumId ON tracks(albumId);
            CREATE INDEX IF NOT EXISTS idx_tracks_userRating ON tracks(userRating);

            CREATE VIRTUAL TABLE IF NOT EXISTS tracks_fts USING FTS5(
                title,
                content='tracks',
                tokenize='unicode61',
                prefix='2,3'
            );

            CREATE TABLE IF NOT EXISTS genres (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE COLLATE NOCASE
            );

            CREATE TABLE IF NOT EXISTS album_genres (
                albumId INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
                genreId INTEGER NOT NULL REFERENCES genres(id) ON DELETE CASCADE,
                PRIMARY KEY (albumId, genreId)
            );
            CREATE INDEX IF NOT EXISTS idx_album_genres_genreId ON album_genres(genreId);",
        )?;

        // Back-compat: add firstGenre to an albums table that predates this
        // column. Pre-existing rows end up with NULL, which sync_albums
        // treats as "unknown — trust updatedAt only", so no blanket
        // re-fetch is triggered at migration time.
        let has_first_genre: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('albums') WHERE name = 'firstGenre'",
            [],
            |r| r.get(0),
        )?;
        if has_first_genre == 0 {
            conn.execute("ALTER TABLE albums ADD COLUMN firstGenre TEXT", [])?;
        }

        Ok(())
    }

    // --- Library query methods ---

    /// Get albums matching any of the given genre names (deduplicated).
    /// Chunks the input to stay within SQLite's bind-parameter limit.
    pub fn albums_for_genres(&self, genre_names: &[&str]) -> Result<Vec<Album>, CacheError> {
        if genre_names.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();

        // SQLite's default SQLITE_MAX_VARIABLE_NUMBER is 999. Chunk to
        // stay well under the limit for any build of SQLite.
        const CHUNK_SIZE: usize = 500;

        if genre_names.len() <= CHUNK_SIZE {
            return Self::albums_for_genres_query(&conn, genre_names);
        }

        // Multiple chunks: collect into a map keyed by sourceId to dedup,
        // then sort to match the single-query ordering.
        let mut seen = std::collections::HashSet::new();
        let mut all = Vec::new();
        for chunk in genre_names.chunks(CHUNK_SIZE) {
            for album in Self::albums_for_genres_query(&conn, chunk)? {
                if seen.insert(album.rating_key.clone()) {
                    all.push(album);
                }
            }
        }
        all.sort_by(|a, b| {
            a.artist_name
                .to_lowercase()
                .cmp(&b.artist_name.to_lowercase())
                .then_with(|| a.year.cmp(&b.year))
        });
        Ok(all)
    }

    fn albums_for_genres_query(
        conn: &rusqlite::Connection,
        genre_names: &[&str],
    ) -> Result<Vec<Album>, CacheError> {
        let placeholders: Vec<String> = (1..=genre_names.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT DISTINCT a.sourceId, a.title, ar.name, a.year, a.artUrl,
                    a.rating, a.studio, a.addedAt, a.lastViewedAt
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             JOIN album_genres ag ON ag.albumId = a.id
             JOIN genres g ON g.id = ag.genreId
             WHERE g.name COLLATE NOCASE IN ({})
             ORDER BY ar.name COLLATE NOCASE, a.year",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = genre_names
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        Self::map_album_rows(&mut stmt, params.as_slice(), conn)
    }

    /// Get a single album by its source_id.
    pub fn album_by_source_id(&self, source_id: &str) -> Result<Option<Album>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl,
                    a.rating, a.studio, a.addedAt, a.lastViewedAt
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE a.sourceId = ?1",
        )?;
        let mut albums = Self::map_album_rows(&mut stmt, params![source_id], &conn)?;
        Ok(albums.pop())
    }

    /// Get all albums for a given year.
    pub fn albums_for_year(&self, year: i32) -> Result<Vec<Album>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl,
                    a.rating, a.studio, a.addedAt, a.lastViewedAt
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE a.year = ?1
             ORDER BY ar.name COLLATE NOCASE, a.title COLLATE NOCASE",
        )?;
        let albums = Self::map_album_rows(&mut stmt, params![year], &conn)?;
        Ok(albums)
    }

    /// Get albums for an artist by artist name.
    pub fn albums_for_artist_name(&self, artist_name: &str) -> Result<Vec<Album>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl,
                    a.rating, a.studio, a.addedAt, a.lastViewedAt
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE ar.name = ?1 COLLATE NOCASE
             ORDER BY a.year",
        )?;
        let albums = Self::map_album_rows(&mut stmt, params![artist_name], &conn)?;
        Ok(albums)
    }

    /// Get albums for an artist by artist source_id.
    pub fn albums_for_artist(&self, artist_source_id: &str) -> Result<Vec<Album>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl,
                    a.rating, a.studio, a.addedAt, a.lastViewedAt
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE ar.sourceId = ?1
             ORDER BY a.year",
        )?;
        let albums = Self::map_album_rows(&mut stmt, params![artist_source_id], &conn)?;
        Ok(albums)
    }

    /// Get a single track by its source_id.
    pub fn track_by_source_id(&self, source_id: &str) -> Result<Option<Track>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT t.sourceId, t.title, ar.name, t.trackArtist,
                    al.title, al.sourceId, t.trackNumber, t.durationMs,
                    t.codec, t.partKey, al.artUrl, t.userRating, t.bitrate, t.discNumber
             FROM tracks t
             JOIN albums al ON al.id = t.albumId
             JOIN artists ar ON ar.id = t.artistId
             WHERE t.sourceId = ?1",
        )?;
        let mut tracks = stmt
            .query_map(params![source_id], Self::map_track_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks.pop())
    }

    /// Get tracks for an album by album source_id.
    pub fn tracks_for_album(&self, album_source_id: &str) -> Result<Vec<Track>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT t.sourceId, t.title, ar.name, t.trackArtist,
                    al.title, al.sourceId, t.trackNumber, t.durationMs,
                    t.codec, t.partKey, al.artUrl, t.userRating, t.bitrate, t.discNumber
             FROM tracks t
             JOIN albums al ON al.id = t.albumId
             JOIN artists ar ON ar.id = t.artistId
             WHERE al.sourceId = ?1
             ORDER BY t.discNumber, t.trackNumber",
        )?;
        let tracks = stmt
            .query_map(params![album_source_id], Self::map_track_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks)
    }

    /// Get all favourite tracks (userRating >= 10).
    pub fn favourite_tracks(&self) -> Result<Vec<Track>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT t.sourceId, t.title, ar.name, t.trackArtist,
                    al.title, al.sourceId, t.trackNumber, t.durationMs,
                    t.codec, t.partKey, al.artUrl, t.userRating, t.bitrate, t.discNumber
             FROM tracks t
             JOIN albums al ON al.id = t.albumId
             JOIN artists ar ON ar.id = t.artistId
             WHERE t.userRating >= 10.0
             ORDER BY ar.name COLLATE NOCASE, al.year, t.discNumber, t.trackNumber",
        )?;
        let tracks = stmt
            .query_map(params![], Self::map_track_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks)
    }

    /// Get all favourite albums (rating >= 10).
    pub fn favourite_albums(&self) -> Result<Vec<Album>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl,
                    a.rating, a.studio, a.addedAt, a.lastViewedAt
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE a.rating >= 10.0
             ORDER BY ar.name COLLATE NOCASE, a.year",
        )?;
        let albums = Self::map_album_rows(&mut stmt, params![], &conn)?;
        Ok(albums)
    }

    /// Get all albums.
    pub fn all_albums(&self) -> Result<Vec<Album>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl,
                    a.rating, a.studio, a.addedAt, a.lastViewedAt
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             ORDER BY ar.name COLLATE NOCASE, a.year",
        )?;
        let albums = Self::map_album_rows(&mut stmt, params![], &conn)?;
        Ok(albums)
    }

    /// Get all artists: (id, name, sourceId, artUrl).
    pub fn all_artists(&self) -> Result<Vec<ArtistRow>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, name, sourceId, artUrl FROM artists ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Random album.
    pub fn random_album(&self) -> Result<Option<Album>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl,
                    a.rating, a.studio, a.addedAt, a.lastViewedAt
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             ORDER BY RANDOM() LIMIT 1",
        )?;
        let mut albums = Self::map_album_rows(&mut stmt, params![], &conn)?;
        Ok(albums.pop())
    }

    /// Get albums by a set of internal (rowid) IDs. Returns full Album objects.
    pub fn albums_by_internal_ids(&self, ids: &HashSet<i64>) -> Result<Vec<Album>, CacheError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();

        const CHUNK_SIZE: usize = 500;
        let id_vec: Vec<i64> = ids.iter().copied().collect();
        let mut all = Vec::new();

        for chunk in id_vec.chunks(CHUNK_SIZE) {
            let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl,
                        a.rating, a.studio, a.addedAt, a.lastViewedAt
                 FROM albums a
                 JOIN artists ar ON ar.id = a.artistId
                 WHERE a.id IN ({})
                 ORDER BY ar.name COLLATE NOCASE, a.year",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let params: Vec<Box<dyn rusqlite::types::ToSql>> =
                chunk.iter().map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>).collect();
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let albums = Self::map_album_rows(&mut stmt, param_refs.as_slice(), &conn)?;
            all.extend(albums);
        }

        if id_vec.len() > CHUNK_SIZE {
            // Multi-chunk: re-sort to get consistent ordering
            all.sort_by(|a, b| {
                a.artist_name
                    .to_lowercase()
                    .cmp(&b.artist_name.to_lowercase())
                    .then_with(|| a.year.cmp(&b.year))
            });
        }
        Ok(all)
    }

    /// Cache statistics.
    pub fn cache_stats(&self) -> Result<CacheStats, CacheError> {
        let conn = self.conn.lock();
        Ok(CacheStats {
            artist_count: conn
                .query_row("SELECT COUNT(*) FROM artists", [], |r| r.get(0))?,
            album_count: conn
                .query_row("SELECT COUNT(*) FROM albums", [], |r| r.get(0))?,
            track_count: conn
                .query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?,
            genre_count: conn
                .query_row("SELECT COUNT(*) FROM genres", [], |r| r.get(0))?,
        })
    }

    /// Update album rating (for favourite toggle).
    pub fn update_album_rating(
        &self,
        source_id: &str,
        rating: Option<f64>,
    ) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE albums SET rating = ?1 WHERE sourceId = ?2",
            params![rating, source_id],
        )?;
        Ok(())
    }

    /// Update track rating (for favourite toggle).
    pub fn update_track_rating(
        &self,
        source_id: &str,
        rating: Option<f64>,
    ) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE tracks SET userRating = ?1 WHERE sourceId = ?2",
            params![rating, source_id],
        )?;
        Ok(())
    }

    // --- Row mapping helpers ---

    pub(super) fn map_album_rows(
        stmt: &mut rusqlite::Statement,
        params: impl rusqlite::Params,
        conn: &Connection,
    ) -> Result<Vec<Album>, CacheError> {
        let _ = conn; // stmt borrows conn implicitly
        let albums = stmt
            .query_map(params, |row| {
                let rating: Option<f64> = row.get(5)?;
                Ok(Album {
                    rating_key: row.get(0)?,
                    title: row.get(1)?,
                    artist_name: row.get(2)?,
                    year: row.get(3)?,
                    thumb: row.get(4)?,
                    genres: Vec::new(), // populated separately if needed
                    is_favourite: rating.map(|r| r >= 10.0).unwrap_or(false),
                    studio: row.get(6)?,
                    added_at: row.get(7)?,
                    last_viewed_at: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(albums)
    }

    /// Map a standard 14-column track row to a Track struct.
    pub(super) fn map_track_row(row: &rusqlite::Row) -> rusqlite::Result<Track> {
        let rating: Option<f64> = row.get(11)?;
        Ok(Track {
            rating_key: row.get(0)?,
            title: row.get(1)?,
            artist_name: row.get(2)?,
            track_artist: row.get(3)?,
            album_title: row.get(4)?,
            album_key: row.get(5)?,
            index: row.get(6)?,
            duration: row
                .get::<_, Option<i64>>(7)?
                .map(|ms| ms as f64 / 1000.0)
                .unwrap_or(0.0),
            codec: row.get(8)?,
            part_key: row.get(9)?,
            thumb: row.get(10)?,
            is_favourite: rating.map(|r| r >= 10.0).unwrap_or(false),
            bitrate: row.get(12)?,
            disc_number: row.get(13)?,
        })
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::upsert::{AlbumUpsertRow, TrackUpsertRow};
    use crate::models::RangeOp;

    fn setup() -> CacheDatabase {
        CacheDatabase::open_in_memory().unwrap()
    }

    fn seed_artist(db: &CacheDatabase, source_id: &str, name: &str) -> i64 {
        let map = db
            .batch_upsert_artists(&[(
                name.into(),
                None,
                source_id.into(),
                None,
                None,
                Some(1000),
            )])
            .unwrap();
        *map.get(source_id).unwrap()
    }

    fn seed_album(
        db: &CacheDatabase,
        source_id: &str,
        title: &str,
        artist_id: i64,
        year: Option<i32>,
    ) -> i64 {
        let map = db
            .batch_upsert_albums(&[AlbumUpsertRow {
                title: title.into(),
                artist_id,
                year,
                source_id: source_id.into(),
                art_url: None,
                updated_at: Some(1000),
                added_at: Some(900),
                last_viewed_at: None,
                first_genre: None,
            }])
            .unwrap();
        *map.get(source_id).unwrap()
    }

    fn seed_track(
        db: &CacheDatabase,
        source_id: &str,
        title: &str,
        album_id: i64,
        artist_id: i64,
    ) {
        db.batch_upsert_tracks(&[TrackUpsertRow {
            title: title.into(),
            album_id,
            artist_id,
            track_number: Some(1),
            disc_number: Some(1),
            duration_ms: Some(240000),
            source_id: source_id.into(),
            codec: Some("flac".into()),
            part_key: None,
            stream_id: None,
            user_rating: None,
            bitrate: Some(1411),
            track_artist: None,
            updated_at: Some(1000),
        }])
        .unwrap();
    }

    // --- CRUD tests ---

    #[test]
    fn test_artist_crud() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        assert!(artist_id > 0);

        assert_eq!(db.artist_id("ar1").unwrap(), Some(artist_id));
        assert_eq!(db.artist_id("nonexistent").unwrap(), None);

        let map = db
            .batch_upsert_artists(&[(
                "Radiohead (Updated)".into(),
                None,
                "ar1".into(),
                None,
                None,
                Some(2000),
            )])
            .unwrap();
        assert_eq!(*map.get("ar1").unwrap(), artist_id);

        let ts = db.all_artist_timestamps().unwrap();
        assert_eq!(ts.get("ar1").unwrap().updated_at, Some(2000));
    }

    #[test]
    fn test_album_crud() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        let album_id = seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));
        assert!(album_id > 0);

        assert_eq!(db.album_id("al1").unwrap(), Some(album_id));

        // Upsert preserves rating via COALESCE
        db.update_album_rating("al1", Some(10.0)).unwrap();
        let _ = db
            .batch_upsert_albums(&[AlbumUpsertRow {
                title: "OK Computer (remaster)".into(),
                artist_id,
                year: Some(1997),
                source_id: "al1".into(),
                art_url: None,
                updated_at: Some(2000),
                added_at: Some(900),
                last_viewed_at: None,
                first_genre: None,
            }])
            .unwrap();

        let favs = db.favourite_albums().unwrap();
        assert_eq!(favs.len(), 1);
        assert_eq!(favs[0].title, "OK Computer (remaster)");
    }

    #[test]
    fn test_track_crud() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        let album_id = seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));
        seed_track(&db, "tr1", "Paranoid Android", album_id, artist_id);

        let tracks = db.tracks_for_album("al1").unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "Paranoid Android");
        assert_eq!(tracks[0].codec, Some("flac".into()));
        assert_eq!(tracks[0].duration, 240.0);
    }

    #[test]
    fn test_multiple_genres_per_album() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        let album_id = seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));

        let rock_id = db.upsert_genre("Rock").unwrap();
        let alt_id = db.upsert_genre("Alternative Rock").unwrap();
        db.link_album_genre(album_id, rock_id).unwrap();
        db.link_album_genre(album_id, alt_id).unwrap();

        let genres = db.album_genres("al1").unwrap();
        assert_eq!(genres.len(), 2);
        assert!(genres.contains(&"Alternative Rock".into()));
        assert!(genres.contains(&"Rock".into()));
    }

    #[test]
    fn test_genre_upsert_case_insensitivity() {
        let db = setup();
        let id1 = db.upsert_genre("Rock").unwrap();
        let id2 = db.upsert_genre("rock").unwrap();
        let id3 = db.upsert_genre("ROCK").unwrap();
        assert_eq!(id1, id2);
        assert_eq!(id2, id3);
    }

    #[test]
    fn test_batch_operations() {
        let db = setup();

        let mut artist_items = Vec::new();
        for i in 0..600 {
            artist_items.push((
                format!("Artist {}", i),
                None,
                format!("ar{}", i),
                None,
                None,
                Some(1000i64),
            ));
        }
        let artist_map = db.batch_upsert_artists(&artist_items).unwrap();
        assert_eq!(artist_map.len(), 600);

        let first_artist_id = *artist_map.get("ar0").unwrap();
        let mut album_items = Vec::new();
        for i in 0..600 {
            album_items.push(AlbumUpsertRow {
                title: format!("Album {}", i),
                artist_id: first_artist_id,
                year: Some(2000 + (i % 20)),
                source_id: format!("al{}", i),
                art_url: None,
                updated_at: Some(1000),
                added_at: None,
                last_viewed_at: None,
                first_genre: None,
            });
        }
        let album_map = db.batch_upsert_albums(&album_items).unwrap();
        assert_eq!(album_map.len(), 600);

        let stats = db.cache_stats().unwrap();
        assert_eq!(stats.artist_count, 600);
        assert_eq!(stats.album_count, 600);
    }

    #[test]
    fn test_timestamp_queries() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        let album_id = seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));
        seed_track(&db, "tr1", "Paranoid Android", album_id, artist_id);

        let artist_ts = db.all_artist_timestamps().unwrap();
        assert_eq!(artist_ts.len(), 1);
        assert_eq!(artist_ts.get("ar1").unwrap().updated_at, Some(1000));

        let album_ts = db.all_album_timestamps().unwrap();
        assert_eq!(album_ts.len(), 1);
        assert_eq!(album_ts.get("al1").unwrap().updated_at, Some(1000));

        let track_ts = db.all_track_timestamps().unwrap();
        assert_eq!(track_ts.len(), 1);
        assert_eq!(track_ts.get("tr1").unwrap().updated_at, Some(1000));
    }

    #[test]
    fn test_fts5_prefix_search() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        let album_id = seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));
        seed_track(&db, "tr1", "Paranoid Android", album_id, artist_id);
        seed_track(&db, "tr2", "Karma Police", album_id, artist_id);

        let results = db.search_tracks_fts("par").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Paranoid Android");
    }

    #[test]
    fn test_album_year_range_filters() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));
        seed_album(&db, "al2", "Kid A", artist_id, Some(2000));
        seed_album(&db, "al3", "In Rainbows", artist_id, Some(2007));
        seed_album(&db, "al4", "No Year Album", artist_id, None);

        let r = db.albums_by_year_range(RangeOp::Equal, 1997).unwrap();
        assert_eq!(r.len(), 1);
        let r = db.albums_by_year_range(RangeOp::GreaterThan, 1997).unwrap();
        assert_eq!(r.len(), 2);
        let r = db.albums_by_year_range(RangeOp::LessThan, 2007).unwrap();
        assert_eq!(r.len(), 2);
        let r = db.albums_by_year_range(RangeOp::GreaterOrEqual, 2000).unwrap();
        assert_eq!(r.len(), 2);
        let r = db.albums_by_year_range(RangeOp::LessOrEqual, 2000).unwrap();
        assert_eq!(r.len(), 2);
        let r = db.albums_by_year_range(RangeOp::GreaterThan, 2010).unwrap();
        assert!(r.is_empty());
        let r = db.albums_by_year_range(RangeOp::GreaterOrEqual, 1990).unwrap();
        assert_eq!(r.len(), 3);
        let r = db.albums_by_year_range(RangeOp::Equal, 2000).unwrap();
        assert_eq!(r.len(), 1);
        let r = db.albums_by_year_range(RangeOp::LessThan, 1990).unwrap();
        assert!(r.is_empty());
        let r = db.albums_by_year_range(RangeOp::GreaterThan, 2007).unwrap();
        assert!(r.is_empty());
        let r = db.albums_by_year_range(RangeOp::Equal, 1999).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn test_set_album_genres_replaces() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        let album_id = seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));

        let rock_id = db.upsert_genre("Rock").unwrap();
        let alt_id = db.upsert_genre("Alternative").unwrap();
        let elec_id = db.upsert_genre("Electronic").unwrap();

        db.set_album_genres(album_id, &[rock_id, alt_id]).unwrap();
        let g = db.album_genres("al1").unwrap();
        assert_eq!(g.len(), 2);

        db.set_album_genres(album_id, &[elec_id]).unwrap();
        let g = db.album_genres("al1").unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0], "Electronic");
    }

    #[test]
    fn test_genre_album_sets() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        let al1 = seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));
        let al2 = seed_album(&db, "al2", "Kid A", artist_id, Some(2000));

        let rock_id = db.upsert_genre("Rock").unwrap();
        let elec_id = db.upsert_genre("Electronic").unwrap();

        db.link_album_genre(al1, rock_id).unwrap();
        db.link_album_genre(al2, rock_id).unwrap();
        db.link_album_genre(al2, elec_id).unwrap();

        let sets = db.genre_album_sets().unwrap();
        assert_eq!(sets.get("Rock").unwrap().len(), 2);
        assert_eq!(sets.get("Electronic").unwrap().len(), 1);
    }

    #[test]
    fn test_search_albums_by_title() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));
        seed_album(&db, "al2", "Kid A", artist_id, Some(2000));

        let results = db.search_albums_by_title("computer").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "OK Computer");

        let results = db.search_albums_by_title("nonexistent").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_update_deep_metadata() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        let album_id = seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));

        db.update_album_deep_metadata(
            album_id,
            &["Rock".into()],
            Some(8.0),
            Some("Parlophone"),
            Some(r##"{"topLeft":"#fff"}"##),
        )
        .unwrap();

        let albums = db.all_albums().unwrap();
        assert_eq!(albums[0].studio, Some("Parlophone".into()));

        let genres = db.album_genres("al1").unwrap();
        assert_eq!(genres, vec!["Rock"]);
    }

    #[test]
    fn test_deep_metadata_preserves_existing_with_coalesce() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        let album_id = seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));

        // First: set studio
        db.update_album_deep_metadata(
            album_id,
            &["Rock".into()],
            Some(8.0),
            Some("Parlophone"),
            None,
        )
        .unwrap();

        // Second: update with None studio — should preserve "Parlophone"
        db.update_album_deep_metadata(
            album_id,
            &["Rock".into()],
            None,
            None,
            None,
        )
        .unwrap();

        let albums = db.all_albums().unwrap();
        assert_eq!(albums[0].studio, Some("Parlophone".into()));
    }

    #[test]
    fn test_random_album() {
        let db = setup();
        assert!(db.random_album().unwrap().is_none());

        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));

        let album = db.random_album().unwrap();
        assert!(album.is_some());
        assert_eq!(album.unwrap().title, "OK Computer");
    }
}
