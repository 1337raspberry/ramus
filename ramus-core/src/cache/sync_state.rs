use std::collections::HashMap;

use rusqlite::params;

use super::db::{CacheDatabase, CachedAlbumInfo, CachedItemInfo, CacheError};

impl CacheDatabase {
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
        // `firstGenre` is the API-order first genre captured at shallow-sync
        // time. `sync_albums` compares it against the next sync's API-order
        // first genre to catch genre-only edits. Do not substitute
        // `MIN(g.name)` — that is alphabetical order, and mismatches would
        // trigger false-positive re-fetches.
        let mut stmt = conn.prepare(
            "SELECT sourceId, id, updatedAt, firstGenre FROM albums",
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
}
