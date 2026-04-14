use std::collections::{HashMap, HashSet};

use rusqlite::params;

use super::db::{CacheDatabase, CacheError};

impl CacheDatabase {
    /// Batch upsert genres and album ↔ genre links. `items` is `(album_id, genre_names)`.
    pub fn batch_upsert_genres_and_links(
        &self,
        items: &[(i64, Vec<String>)],
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

    /// Upsert a genre name case-insensitively and return its id.
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

    /// Link an album to a genre. Duplicate links are ignored.
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

    /// Atomically update album deep metadata (genres by name, rating, studio, colors).
    /// `COALESCE` preserves existing values when new values are `NULL`.
    pub fn update_album_deep_metadata(
        &self,
        album_id: i64,
        genre_names: &[String],
        rating: Option<f64>,
        studio: Option<&str>,
        colors_json: Option<&str>,
    ) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;

        tx.execute(
            "UPDATE albums SET
                rating = COALESCE(?1, albums.rating),
                studio = COALESCE(?2, albums.studio),
                ultraBlurColors = COALESCE(?3, albums.ultraBlurColors)
             WHERE id = ?4",
            params![rating, studio, colors_json, album_id],
        )?;

        tx.execute(
            "DELETE FROM album_genres WHERE albumId = ?1",
            params![album_id],
        )?;
        {
            let mut genre_stmt = tx.prepare_cached(
                "INSERT INTO genres (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            )?;
            let mut id_stmt =
                tx.prepare_cached("SELECT id FROM genres WHERE name = ?1 COLLATE NOCASE")?;
            let mut link_stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO album_genres (albumId, genreId) VALUES (?1, ?2)",
            )?;

            for name in genre_names {
                genre_stmt.execute(params![name])?;
                let genre_id: i64 = id_stmt.query_row(params![name], |r| r.get(0))?;
                link_stmt.execute(params![album_id, genre_id])?;
            }
        }

        tx.commit()?;
        Ok(())
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

    /// Album IDs tagged with any of the given genre names.
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

    /// Album IDs for favourited albums (rating >= 10).
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
}
