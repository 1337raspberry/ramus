use std::collections::{HashMap, HashSet};
use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{params, Connection};

use crate::models::{Album, RangeOp, Track, UltraBlurColors};

// ---------------------------------------------------------------------------
// Type aliases for complex tuples
// ---------------------------------------------------------------------------

/// (name, sortName, sourceId, artUrl, summary, updatedAt)
pub type ArtistUpsertRow = (String, Option<String>, String, Option<String>, Option<String>, Option<i64>);

/// (id, name, sourceId, artUrl)
pub type ArtistRow = (i64, String, String, Option<String>);

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Helper types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Search result row types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// CacheDatabase
// ---------------------------------------------------------------------------

pub struct CacheDatabase {
    conn: Mutex<Connection>,
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
             PRAGMA synchronous = NORMAL;",
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
                addedAt INTEGER,
                lastViewedAt INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_albums_title ON albums(title);

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
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Batch Upserts (for sync)
    // -----------------------------------------------------------------------

    /// Batch upsert artists. Returns sourceId → local id map.
    pub fn batch_upsert_artists(
        &self,
        items: &[ArtistUpsertRow],
    ) -> Result<HashMap<String, i64>, CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        let mut map = HashMap::new();

        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO artists (name, sortName, sourceId, artUrl, summary, updatedAt)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(sourceId) DO UPDATE SET
                     name = excluded.name,
                     sortName = excluded.sortName,
                     artUrl = excluded.artUrl,
                     summary = excluded.summary,
                     updatedAt = excluded.updatedAt",
            )?;

            for chunk in items.chunks(500) {
                for (name, sort_name, source_id, art_url, summary, updated_at) in chunk {
                    stmt.execute(params![name, sort_name, source_id, art_url, summary, updated_at])?;
                }
            }
        }

        // Collect ID map
        {
            let mut id_stmt = tx.prepare_cached("SELECT sourceId, id FROM artists")?;
            let rows = id_stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (sid, id) = row?;
                map.insert(sid, id);
            }
        }

        tx.commit()?;
        Ok(map)
    }

    /// Batch upsert albums. Returns sourceId → local id map.
    /// COALESCE preserves existing rating/studio/colors.
    pub fn batch_upsert_albums(
        &self,
        items: &[AlbumUpsertRow],
    ) -> Result<HashMap<String, i64>, CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        let mut map = HashMap::new();

        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO albums (title, artistId, year, sourceId, artUrl, updatedAt, addedAt, lastViewedAt)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(sourceId) DO UPDATE SET
                     title = excluded.title,
                     artistId = excluded.artistId,
                     year = excluded.year,
                     artUrl = excluded.artUrl,
                     rating = COALESCE(excluded.rating, albums.rating),
                     studio = COALESCE(excluded.studio, albums.studio),
                     updatedAt = excluded.updatedAt,
                     addedAt = COALESCE(excluded.addedAt, albums.addedAt),
                     lastViewedAt = COALESCE(excluded.lastViewedAt, albums.lastViewedAt)",
            )?;

            for chunk in items.chunks(500) {
                for row in chunk {
                    stmt.execute(params![
                        row.title,
                        row.artist_id,
                        row.year,
                        row.source_id,
                        row.art_url,
                        row.updated_at,
                        row.added_at,
                        row.last_viewed_at,
                    ])?;
                }
            }
        }

        {
            let mut id_stmt = tx.prepare_cached("SELECT sourceId, id FROM albums")?;
            let rows = id_stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (sid, id) = row?;
                map.insert(sid, id);
            }
        }

        tx.commit()?;
        Ok(map)
    }

    /// Batch upsert tracks. Also rebuilds FTS5 entries.
    pub fn batch_upsert_tracks(&self, items: &[TrackUpsertRow]) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;

        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO tracks (title, albumId, artistId, trackNumber, discNumber, durationMs, sourceId, codec, partKey, streamId, userRating, bitrate, trackArtist, updatedAt)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(sourceId) DO UPDATE SET
                     title = excluded.title,
                     albumId = excluded.albumId,
                     artistId = excluded.artistId,
                     trackNumber = excluded.trackNumber,
                     discNumber = excluded.discNumber,
                     durationMs = excluded.durationMs,
                     codec = excluded.codec,
                     partKey = excluded.partKey,
                     streamId = excluded.streamId,
                     userRating = excluded.userRating,
                     bitrate = excluded.bitrate,
                     trackArtist = excluded.trackArtist,
                     updatedAt = excluded.updatedAt",
            )?;

            let mut fts_stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO tracks_fts(rowid, title) VALUES (?1, ?2)",
            )?;

            for chunk in items.chunks(500) {
                for row in chunk {
                    stmt.execute(params![
                        row.title,
                        row.album_id,
                        row.artist_id,
                        row.track_number,
                        row.disc_number,
                        row.duration_ms,
                        row.source_id,
                        row.codec,
                        row.part_key,
                        row.stream_id,
                        row.user_rating,
                        row.bitrate,
                        row.track_artist,
                        row.updated_at,
                    ])?;

                    // Get the rowid (last inserted or existing)
                    let track_id: i64 = tx.query_row(
                        "SELECT id FROM tracks WHERE sourceId = ?1",
                        params![row.source_id],
                        |r| r.get(0),
                    )?;
                    fts_stmt.execute(params![track_id, row.title])?;
                }
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Batch upsert genres and album↔genre links.
    pub fn batch_upsert_genres_and_links(
        &self,
        items: &[(i64, Vec<String>)], // (album_id, genre_names)
    ) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;

        {
            let mut genre_stmt = tx.prepare_cached(
                "INSERT INTO genres (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            )?;
            let mut id_stmt =
                tx.prepare_cached("SELECT id FROM genres WHERE name = ?1 COLLATE NOCASE")?;
            let mut link_stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO album_genres (albumId, genreId) VALUES (?1, ?2)",
            )?;

            for (album_id, genre_names) in items {
                for name in genre_names {
                    genre_stmt.execute(params![name])?;
                    let genre_id: i64 = id_stmt.query_row(params![name], |r| r.get(0))?;
                    link_stmt.execute(params![album_id, genre_id])?;
                }
            }
        }

        tx.commit()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Genre Management
    // -----------------------------------------------------------------------

    /// Upsert a genre name (case-insensitive). Returns the genre id.
    pub fn upsert_genre(&self, name: &str) -> Result<i64, CacheError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO genres (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            params![name],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM genres WHERE name = ?1 COLLATE NOCASE",
            params![name],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    /// Link an album to a genre. INSERT OR IGNORE.
    pub fn link_album_genre(&self, album_id: i64, genre_id: i64) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR IGNORE INTO album_genres (albumId, genreId) VALUES (?1, ?2)",
            params![album_id, genre_id],
        )?;
        Ok(())
    }

    /// Replace all genres for an album.
    pub fn set_album_genres(&self, album_id: i64, genre_ids: &[i64]) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM album_genres WHERE albumId = ?1",
            params![album_id],
        )?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO album_genres (albumId, genreId) VALUES (?1, ?2)",
            )?;
            for gid in genre_ids {
                stmt.execute(params![album_id, gid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Atomically update album deep metadata (genres, rating, studio, colors).
    pub fn update_album_deep_metadata(
        &self,
        album_id: i64,
        genre_ids: &[i64],
        rating: Option<f64>,
        studio: Option<&str>,
        colors_json: Option<&str>,
    ) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;

        tx.execute(
            "UPDATE albums SET rating = ?1, studio = ?2, ultraBlurColors = ?3 WHERE id = ?4",
            params![rating, studio, colors_json, album_id],
        )?;

        tx.execute(
            "DELETE FROM album_genres WHERE albumId = ?1",
            params![album_id],
        )?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO album_genres (albumId, genreId) VALUES (?1, ?2)",
            )?;
            for gid in genre_ids {
                stmt.execute(params![album_id, gid])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Timestamp lookups (for incremental sync)
    // -----------------------------------------------------------------------

    pub fn all_artist_timestamps(&self) -> Result<HashMap<String, CachedItemInfo>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT sourceId, id, updatedAt FROM artists")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                CachedItemInfo {
                    id: row.get(1)?,
                    updated_at: row.get(2)?,
                },
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (sid, info) = row?;
            map.insert(sid, info);
        }
        Ok(map)
    }

    pub fn all_album_timestamps(&self) -> Result<HashMap<String, CachedAlbumInfo>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, a.id, a.updatedAt,
                    (SELECT g.name FROM album_genres ag
                     JOIN genres g ON g.id = ag.genreId
                     WHERE ag.albumId = a.id LIMIT 1)
             FROM albums a",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                CachedAlbumInfo {
                    id: row.get(1)?,
                    updated_at: row.get(2)?,
                    first_genre: row.get(3)?,
                },
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (sid, info) = row?;
            map.insert(sid, info);
        }
        Ok(map)
    }

    pub fn all_track_timestamps(&self) -> Result<HashMap<String, CachedItemInfo>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT sourceId, id, updatedAt FROM tracks")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                CachedItemInfo {
                    id: row.get(1)?,
                    updated_at: row.get(2)?,
                },
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (sid, info) = row?;
            map.insert(sid, info);
        }
        Ok(map)
    }

    // -----------------------------------------------------------------------
    // ID lookups
    // -----------------------------------------------------------------------

    pub fn artist_id(&self, source_id: &str) -> Result<Option<i64>, CacheError> {
        let conn = self.conn.lock();
        let r = conn.query_row(
            "SELECT id FROM artists WHERE sourceId = ?1",
            params![source_id],
            |row| row.get(0),
        );
        match r {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn album_id(&self, source_id: &str) -> Result<Option<i64>, CacheError> {
        let conn = self.conn.lock();
        let r = conn.query_row(
            "SELECT id FROM albums WHERE sourceId = ?1",
            params![source_id],
            |row| row.get(0),
        );
        match r {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn album_updated_at(&self, source_id: &str) -> Result<Option<i64>, CacheError> {
        let conn = self.conn.lock();
        let r = conn.query_row(
            "SELECT updatedAt FROM albums WHERE sourceId = ?1",
            params![source_id],
            |row| row.get(0),
        );
        match r {
            Ok(ts) => Ok(ts),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    // -----------------------------------------------------------------------
    // Query methods
    // -----------------------------------------------------------------------

    /// Get albums for a genre name (joined with artist).
    pub fn albums_for_genre(&self, genre_name: &str) -> Result<Vec<Album>, CacheError> {
        self.albums_for_genres(&[genre_name])
    }

    /// Get albums matching any of the given genre names (deduplicated).
    pub fn albums_for_genres(&self, genre_names: &[&str]) -> Result<Vec<Album>, CacheError> {
        if genre_names.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock();
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
        let albums = Self::map_album_rows(&mut stmt, params.as_slice(), &conn)?;
        Ok(albums)
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
            .query_map(params![album_source_id], |row| {
                let rating: Option<f64> = row.get(11)?;
                Ok(Track {
                    rating_key: row.get(0)?,
                    title: row.get(1)?,
                    artist_name: row.get(2)?,
                    track_artist: row.get(3)?,
                    album_title: row.get(4)?,
                    album_key: row.get(5)?,
                    index: row.get(6)?,
                    duration: row.get::<_, Option<i64>>(7)?
                        .map(|ms| ms as f64 / 1000.0)
                        .unwrap_or(0.0),
                    codec: row.get(8)?,
                    part_key: row.get(9)?,
                    thumb: row.get(10)?,
                    is_favourite: rating.map(|r| r >= 10.0).unwrap_or(false),
                    bitrate: row.get(12)?,
                    disc_number: row.get(13)?,
                })
            })?
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
            .query_map(params![], |row| {
                let rating: Option<f64> = row.get(11)?;
                Ok(Track {
                    rating_key: row.get(0)?,
                    title: row.get(1)?,
                    artist_name: row.get(2)?,
                    track_artist: row.get(3)?,
                    album_title: row.get(4)?,
                    album_key: row.get(5)?,
                    index: row.get(6)?,
                    duration: row.get::<_, Option<i64>>(7)?
                        .map(|ms| ms as f64 / 1000.0)
                        .unwrap_or(0.0),
                    codec: row.get(8)?,
                    part_key: row.get(9)?,
                    thumb: row.get(10)?,
                    is_favourite: rating.map(|r| r >= 10.0).unwrap_or(false),
                    bitrate: row.get(12)?,
                    disc_number: row.get(13)?,
                })
            })?
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
    pub fn all_artists(
        &self,
    ) -> Result<Vec<ArtistRow>, CacheError> {
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

    /// Genre name → set of album IDs.
    pub fn genre_album_sets(&self) -> Result<HashMap<String, HashSet<i64>>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT g.name, ag.albumId
             FROM album_genres ag
             JOIN genres g ON g.id = ag.genreId",
        )?;
        let mut map: HashMap<String, HashSet<i64>> = HashMap::new();
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (name, album_id) = row?;
            map.entry(name).or_default().insert(album_id);
        }
        Ok(map)
    }

    /// FTS5 prefix search on track titles.
    pub fn search_tracks_fts(&self, query: &str) -> Result<Vec<Track>, CacheError> {
        let conn = self.conn.lock();
        let fts_query = format!("{}*", escape_fts5(query));
        let mut stmt = conn.prepare(
            "SELECT t.sourceId, t.title, ar.name, t.trackArtist,
                    al.title, al.sourceId, t.trackNumber, t.durationMs,
                    t.codec, t.partKey, al.artUrl, t.userRating, t.bitrate, t.discNumber
             FROM tracks_fts fts
             JOIN tracks t ON t.id = fts.rowid
             JOIN albums al ON al.id = t.albumId
             JOIN artists ar ON ar.id = t.artistId
             WHERE tracks_fts MATCH ?1
             ORDER BY rank",
        )?;
        let tracks = stmt
            .query_map(params![fts_query], |row| {
                let rating: Option<f64> = row.get(11)?;
                Ok(Track {
                    rating_key: row.get(0)?,
                    title: row.get(1)?,
                    artist_name: row.get(2)?,
                    track_artist: row.get(3)?,
                    album_title: row.get(4)?,
                    album_key: row.get(5)?,
                    index: row.get(6)?,
                    duration: row.get::<_, Option<i64>>(7)?
                        .map(|ms| ms as f64 / 1000.0)
                        .unwrap_or(0.0),
                    codec: row.get(8)?,
                    part_key: row.get(9)?,
                    thumb: row.get(10)?,
                    is_favourite: rating.map(|r| r >= 10.0).unwrap_or(false),
                    bitrate: row.get(12)?,
                    disc_number: row.get(13)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks)
    }

    /// Search albums by title (LIKE contains).
    pub fn search_albums_by_title(&self, query: &str) -> Result<Vec<Album>, CacheError> {
        let conn = self.conn.lock();
        let pattern = format!("%{}%", escape_like(query));
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl,
                    a.rating, a.studio, a.addedAt, a.lastViewedAt
             FROM albums a
             JOIN artists ar ON ar.id = a.artistId
             WHERE a.title LIKE ?1 ESCAPE '\\'
             ORDER BY a.title COLLATE NOCASE",
        )?;
        let albums = Self::map_album_rows(&mut stmt, params![pattern], &conn)?;
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

    // -----------------------------------------------------------------------
    // Search query methods (used by SearchEngine)
    // -----------------------------------------------------------------------

    /// Search albums by title (LIKE contains), with optional album ID filter.
    pub fn search_albums_by_title_filtered(
        &self,
        query: &str,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<AlbumSearchRow>, CacheError> {
        let conn = self.conn.lock();
        let pattern = format!("%{}%", escape_like(query));

        if let Some(ids) = album_ids {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let placeholders = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
                 FROM albums a
                 JOIN artists ar ON ar.id = a.artistId
                 WHERE a.title LIKE ?1 ESCAPE '\\' AND a.id IN ({})
                 ORDER BY a.title COLLATE NOCASE
                 LIMIT ?2",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![pattern, limit as i64], |row| {
                    Ok(AlbumSearchRow {
                        album_source_id: row.get(0)?,
                        album_title: row.get(1)?,
                        artist_name: row.get(2)?,
                        year: row.get(3)?,
                        art_url: row.get(4)?,
                        is_favourite: row.get::<_, Option<f64>>(5)?.map(|r| r >= 10.0).unwrap_or(false),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
                 FROM albums a
                 JOIN artists ar ON ar.id = a.artistId
                 WHERE a.title LIKE ?1 ESCAPE '\\'
                 ORDER BY a.title COLLATE NOCASE
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![pattern, limit as i64], |row| {
                    Ok(AlbumSearchRow {
                        album_source_id: row.get(0)?,
                        album_title: row.get(1)?,
                        artist_name: row.get(2)?,
                        year: row.get(3)?,
                        art_url: row.get(4)?,
                        is_favourite: row.get::<_, Option<f64>>(5)?.map(|r| r >= 10.0).unwrap_or(false),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }

    /// Search albums by artist name (LIKE contains), with optional album ID filter.
    pub fn search_albums_by_artist(
        &self,
        query: &str,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<AlbumSearchRow>, CacheError> {
        let conn = self.conn.lock();
        let pattern = format!("%{}%", escape_like(query));

        if let Some(ids) = album_ids {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let placeholders = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
                 FROM albums a
                 JOIN artists ar ON ar.id = a.artistId
                 WHERE ar.name LIKE ?1 ESCAPE '\\' AND a.id IN ({})
                 ORDER BY ar.name COLLATE NOCASE, a.year
                 LIMIT ?2",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![pattern, limit as i64], |row| {
                    Ok(AlbumSearchRow {
                        album_source_id: row.get(0)?,
                        album_title: row.get(1)?,
                        artist_name: row.get(2)?,
                        year: row.get(3)?,
                        art_url: row.get(4)?,
                        is_favourite: row.get::<_, Option<f64>>(5)?.map(|r| r >= 10.0).unwrap_or(false),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
                 FROM albums a
                 JOIN artists ar ON ar.id = a.artistId
                 WHERE ar.name LIKE ?1 ESCAPE '\\'
                 ORDER BY ar.name COLLATE NOCASE, a.year
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![pattern, limit as i64], |row| {
                    Ok(AlbumSearchRow {
                        album_source_id: row.get(0)?,
                        album_title: row.get(1)?,
                        artist_name: row.get(2)?,
                        year: row.get(3)?,
                        art_url: row.get(4)?,
                        is_favourite: row.get::<_, Option<f64>>(5)?.map(|r| r >= 10.0).unwrap_or(false),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }

    /// Search albums by artist name OR album title (LIKE contains), with optional album ID filter.
    pub fn search_albums_by_artist_or_title(
        &self,
        query: &str,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<AlbumSearchRow>, CacheError> {
        let conn = self.conn.lock();
        let pattern = format!("%{}%", escape_like(query));

        if let Some(ids) = album_ids {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let placeholders = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
                 FROM albums a
                 JOIN artists ar ON ar.id = a.artistId
                 WHERE (ar.name LIKE ?1 ESCAPE '\\' OR a.title LIKE ?1 ESCAPE '\\') AND a.id IN ({})
                 ORDER BY a.title COLLATE NOCASE
                 LIMIT ?2",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![pattern, limit as i64], |row| {
                    Ok(AlbumSearchRow {
                        album_source_id: row.get(0)?,
                        album_title: row.get(1)?,
                        artist_name: row.get(2)?,
                        year: row.get(3)?,
                        art_url: row.get(4)?,
                        is_favourite: row.get::<_, Option<f64>>(5)?.map(|r| r >= 10.0).unwrap_or(false),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
                 FROM albums a
                 JOIN artists ar ON ar.id = a.artistId
                 WHERE ar.name LIKE ?1 ESCAPE '\\' OR a.title LIKE ?1 ESCAPE '\\'
                 ORDER BY a.title COLLATE NOCASE
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![pattern, limit as i64], |row| {
                    Ok(AlbumSearchRow {
                        album_source_id: row.get(0)?,
                        album_title: row.get(1)?,
                        artist_name: row.get(2)?,
                        year: row.get(3)?,
                        art_url: row.get(4)?,
                        is_favourite: row.get::<_, Option<f64>>(5)?.map(|r| r >= 10.0).unwrap_or(false),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }

    /// Search albums by ID filter only (for genre/year/fav filter-only queries), random order.
    pub fn search_albums_filtered(
        &self,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<AlbumSearchRow>, CacheError> {
        let conn = self.conn.lock();
        if let Some(ids) = album_ids {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let placeholders = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT a.sourceId, a.title, ar.name, a.year, a.artUrl, a.rating
                 FROM albums a
                 JOIN artists ar ON ar.id = a.artistId
                 WHERE a.id IN ({})
                 ORDER BY RANDOM()
                 LIMIT ?1",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![limit as i64], |row| {
                    Ok(AlbumSearchRow {
                        album_source_id: row.get(0)?,
                        album_title: row.get(1)?,
                        artist_name: row.get(2)?,
                        year: row.get(3)?,
                        art_url: row.get(4)?,
                        is_favourite: row.get::<_, Option<f64>>(5)?.map(|r| r >= 10.0).unwrap_or(false),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            Ok(Vec::new())
        }
    }

    /// FTS5 enriched track search with album/artist joins.
    pub fn search_tracks_enriched(
        &self,
        fts_query: &str,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<TrackSearchRow>, CacheError> {
        let conn = self.conn.lock();

        if let Some(ids) = album_ids {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let placeholders = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT t.id, t.sourceId, t.title, ar.name, al.title, al.sourceId, al.artUrl, t.trackArtist, t.userRating
                 FROM tracks_fts fts
                 JOIN tracks t ON t.id = fts.rowid
                 JOIN albums al ON al.id = t.albumId
                 JOIN artists ar ON ar.id = t.artistId
                 WHERE tracks_fts MATCH ?1 AND t.albumId IN ({})
                 ORDER BY rank
                 LIMIT ?2",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![fts_query, limit as i64], |row| {
                    Ok(TrackSearchRow {
                        id: row.get(0)?,
                        track_source_id: row.get(1)?,
                        track_title: row.get(2)?,
                        artist_name: row.get(3)?,
                        album_title: row.get(4)?,
                        album_source_id: row.get(5)?,
                        art_url: row.get(6)?,
                        track_artist: row.get(7)?,
                        is_favourite: row.get::<_, Option<f64>>(8)?.map(|r| r >= 10.0).unwrap_or(false),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT t.id, t.sourceId, t.title, ar.name, al.title, al.sourceId, al.artUrl, t.trackArtist, t.userRating
                 FROM tracks_fts fts
                 JOIN tracks t ON t.id = fts.rowid
                 JOIN albums al ON al.id = t.albumId
                 JOIN artists ar ON ar.id = t.artistId
                 WHERE tracks_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(params![fts_query, limit as i64], |row| {
                    Ok(TrackSearchRow {
                        id: row.get(0)?,
                        track_source_id: row.get(1)?,
                        track_title: row.get(2)?,
                        artist_name: row.get(3)?,
                        album_title: row.get(4)?,
                        album_source_id: row.get(5)?,
                        art_url: row.get(6)?,
                        track_artist: row.get(7)?,
                        is_favourite: row.get::<_, Option<f64>>(8)?.map(|r| r >= 10.0).unwrap_or(false),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }

    /// Get all tracks as fuzzy search candidates (id, sourceId, title, artistName, albumTitle, albumSourceId, artUrl, trackArtist).
    pub fn search_candidates(
        &self,
        album_ids: Option<&HashSet<i64>>,
        limit: usize,
    ) -> Result<Vec<TrackSearchRow>, CacheError> {
        let conn = self.conn.lock();

        if let Some(ids) = album_ids {
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let placeholders = ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT t.id, t.sourceId, t.title, ar.name, al.title, al.sourceId, al.artUrl, t.trackArtist, t.userRating
                 FROM tracks t
                 JOIN albums al ON al.id = t.albumId
                 JOIN artists ar ON ar.id = t.artistId
                 WHERE t.albumId IN ({})
                 LIMIT ?1",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![limit as i64], |row| {
                    Ok(TrackSearchRow {
                        id: row.get(0)?,
                        track_source_id: row.get(1)?,
                        track_title: row.get(2)?,
                        artist_name: row.get(3)?,
                        album_title: row.get(4)?,
                        album_source_id: row.get(5)?,
                        art_url: row.get(6)?,
                        track_artist: row.get(7)?,
                        is_favourite: row.get::<_, Option<f64>>(8)?.map(|r| r >= 10.0).unwrap_or(false),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT t.id, t.sourceId, t.title, ar.name, al.title, al.sourceId, al.artUrl, t.trackArtist, t.userRating
                 FROM tracks t
                 JOIN albums al ON al.id = t.albumId
                 JOIN artists ar ON ar.id = t.artistId
                 LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![limit as i64], |row| {
                    Ok(TrackSearchRow {
                        id: row.get(0)?,
                        track_source_id: row.get(1)?,
                        track_title: row.get(2)?,
                        artist_name: row.get(3)?,
                        album_title: row.get(4)?,
                        album_source_id: row.get(5)?,
                        art_url: row.get(6)?,
                        track_artist: row.get(7)?,
                        is_favourite: row.get::<_, Option<f64>>(8)?.map(|r| r >= 10.0).unwrap_or(false),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }

    /// Get album IDs that are tagged with any of the given genre names.
    pub fn album_ids_for_genre_names(
        &self,
        genre_names: &[String],
    ) -> Result<HashSet<i64>, CacheError> {
        if genre_names.is_empty() {
            return Ok(HashSet::new());
        }
        let conn = self.conn.lock();
        let placeholders = genre_names.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT DISTINCT ag.albumId
             FROM album_genres ag
             JOIN genres g ON g.id = ag.genreId
             WHERE g.name IN ({}) COLLATE NOCASE",
            placeholders
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(genre_names.iter()),
            |row| row.get::<_, i64>(0),
        )?;
        let mut set = HashSet::new();
        for row in rows {
            set.insert(row?);
        }
        Ok(set)
    }

    /// Get album IDs for favourited albums (rating >= 10).
    pub fn album_ids_for_favourites(&self) -> Result<HashSet<i64>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id FROM albums WHERE rating IS NOT NULL AND rating >= 10.0",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
        let mut set = HashSet::new();
        for row in rows {
            set.insert(row?);
        }
        Ok(set)
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

    /// UltraBlur colors for an album (parsed from JSON stored in DB).
    pub fn album_colors(&self, source_id: &str) -> Result<Option<UltraBlurColors>, CacheError> {
        let conn = self.conn.lock();
        let r: Result<Option<String>, _> = conn.query_row(
            "SELECT ultraBlurColors FROM albums WHERE sourceId = ?1",
            params![source_id],
            |row| row.get(0),
        );
        match r {
            Ok(Some(json)) => {
                let colors: UltraBlurColors = serde_json::from_str(&json)?;
                Ok(Some(colors))
            }
            Ok(None) => Ok(None),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Genre names for an album.
    pub fn album_genres(&self, source_id: &str) -> Result<Vec<String>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT g.name FROM genres g
             JOIN album_genres ag ON ag.genreId = g.id
             JOIN albums a ON a.id = ag.albumId
             WHERE a.sourceId = ?1
             ORDER BY g.name COLLATE NOCASE",
        )?;
        let names = stmt
            .query_map(params![source_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(names)
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn map_album_rows(
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
}

// ---------------------------------------------------------------------------
// Upsert row types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AlbumUpsertRow {
    pub title: String,
    pub artist_id: i64,
    pub year: Option<i32>,
    pub source_id: String,
    pub art_url: Option<String>,
    pub updated_at: Option<i64>,
    pub added_at: Option<i64>,
    pub last_viewed_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct TrackUpsertRow {
    pub title: String,
    pub album_id: i64,
    pub artist_id: i64,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub duration_ms: Option<i64>,
    pub source_id: String,
    pub codec: Option<String>,
    pub part_key: Option<String>,
    pub stream_id: Option<i32>,
    pub user_rating: Option<f64>,
    pub bitrate: Option<i32>,
    pub track_artist: Option<String>,
    pub updated_at: Option<i64>,
}

// ---------------------------------------------------------------------------
// FTS5 / LIKE escaping
// ---------------------------------------------------------------------------

/// Escape a string for FTS5 MATCH queries.
/// Strip `"*():^{}`, replace `-` with space.
pub fn escape_fts5(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '"' | '*' | '(' | ')' | ':' | '^' | '{' | '}' => {}
            '-' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape a string for SQL LIKE patterns (escape `%`, `_`, `\`).
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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

    // -- CRUD tests --

    #[test]
    fn test_artist_crud() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        assert!(artist_id > 0);

        // Read back
        assert_eq!(db.artist_id("ar1").unwrap(), Some(artist_id));
        assert_eq!(db.artist_id("nonexistent").unwrap(), None);

        // Upsert same source_id with updated name
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
        assert_eq!(*map.get("ar1").unwrap(), artist_id); // same id

        // Verify timestamps
        let ts = db.all_artist_timestamps().unwrap();
        assert_eq!(ts.get("ar1").unwrap().updated_at, Some(2000));
    }

    #[test]
    fn test_album_crud() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        let album_id = seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));
        assert!(album_id > 0);

        // ID lookups
        assert_eq!(db.album_id("al1").unwrap(), Some(album_id));
        assert_eq!(db.album_updated_at("al1").unwrap(), Some(1000));

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
            }])
            .unwrap();

        // Rating should be preserved
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
        assert_eq!(tracks[0].duration, 240.0); // 240000 ms → 240 s
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

        // Batch artists
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

        // Batch albums
        let first_artist_id = *artist_map.get("ar0").unwrap();
        let mut album_items = Vec::new();
        for i in 0..600 {
            album_items.push(AlbumUpsertRow {
                title: format!("Album {}", i),
                artist_id: first_artist_id,
                year: Some(2000 + (i % 20) as i32),
                source_id: format!("al{}", i),
                art_url: None,
                updated_at: Some(1000),
                added_at: None,
                last_viewed_at: None,
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
    fn test_like_pattern_escaping() {
        // % in search term should be treated literally
        assert_eq!(escape_like("100%"), "100\\%");
        // _ should be escaped
        assert_eq!(escape_like("track_1"), "track\\_1");
        // \ should be escaped
        assert_eq!(escape_like("back\\slash"), "back\\\\slash");
        // Normal text passes through
        assert_eq!(escape_like("hello world"), "hello world");
        // Empty string
        assert_eq!(escape_like(""), "");
        // All special chars
        assert_eq!(escape_like("%_\\"), "\\%\\_\\\\");
        // Mixed
        assert_eq!(escape_like("foo%bar_baz"), "foo\\%bar\\_baz");
        // Multiple consecutive
        assert_eq!(escape_like("%%"), "\\%\\%");
        // Unicode preserved
        assert_eq!(escape_like("björk"), "björk");
    }

    #[test]
    fn test_album_year_range_filters() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));
        seed_album(&db, "al2", "Kid A", artist_id, Some(2000));
        seed_album(&db, "al3", "In Rainbows", artist_id, Some(2007));
        // Album with no year (should not appear in results)
        seed_album(&db, "al4", "No Year Album", artist_id, None);

        // Equal
        let r = db.albums_by_year_range(RangeOp::Equal, 1997).unwrap();
        assert_eq!(r.len(), 1);

        // Greater than
        let r = db.albums_by_year_range(RangeOp::GreaterThan, 1997).unwrap();
        assert_eq!(r.len(), 2); // 2000 and 2007

        // Less than
        let r = db.albums_by_year_range(RangeOp::LessThan, 2007).unwrap();
        assert_eq!(r.len(), 2); // 1997 and 2000

        // Greater or equal
        let r = db.albums_by_year_range(RangeOp::GreaterOrEqual, 2000).unwrap();
        assert_eq!(r.len(), 2); // 2000 and 2007

        // Less or equal
        let r = db.albums_by_year_range(RangeOp::LessOrEqual, 2000).unwrap();
        assert_eq!(r.len(), 2); // 1997 and 2000

        // No matches
        let r = db.albums_by_year_range(RangeOp::GreaterThan, 2010).unwrap();
        assert!(r.is_empty());

        // All match
        let r = db.albums_by_year_range(RangeOp::GreaterOrEqual, 1990).unwrap();
        assert_eq!(r.len(), 3); // null year excluded

        // Exact boundary
        let r = db.albums_by_year_range(RangeOp::Equal, 2000).unwrap();
        assert_eq!(r.len(), 1);

        // Less than minimum
        let r = db.albums_by_year_range(RangeOp::LessThan, 1990).unwrap();
        assert!(r.is_empty());

        // Greater than max
        let r = db.albums_by_year_range(RangeOp::GreaterThan, 2007).unwrap();
        assert!(r.is_empty());

        // Equal nonexistent year
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

        // Replace with different genres
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
    fn test_fts5_escaping() {
        assert_eq!(escape_fts5("hello-world"), "hello world");
        assert_eq!(escape_fts5(r#"test"quote"#), "testquote");
        assert_eq!(escape_fts5("foo*bar"), "foobar");
        assert_eq!(escape_fts5("(group)"), "group");
        assert_eq!(escape_fts5("normal text"), "normal text");
        assert_eq!(escape_fts5("colon:value"), "colonvalue");
    }

    #[test]
    fn test_update_deep_metadata() {
        let db = setup();
        let artist_id = seed_artist(&db, "ar1", "Radiohead");
        let album_id = seed_album(&db, "al1", "OK Computer", artist_id, Some(1997));

        let rock_id = db.upsert_genre("Rock").unwrap();
        db.update_album_deep_metadata(
            album_id,
            &[rock_id],
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
