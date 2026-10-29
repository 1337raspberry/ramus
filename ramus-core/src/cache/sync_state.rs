use std::collections::HashMap;

use rusqlite::params;

use super::db::{CacheDatabase, CachedAlbumInfo, CachedItemInfo, CacheError};

/// An album's per-user fields. Plex keeps these per account, so setting them
/// never moves the album's `updatedAt`.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumUserState {
    pub rating: Option<f64>,
    pub view_count: Option<i64>,
    pub last_viewed_at: Option<i64>,
}

impl AlbumUserState {
    /// Equal for sync purposes: an absent play count is zero plays.
    pub fn same_as(&self, other: &Self) -> bool {
        self.rating == other.rating
            && self.view_count.unwrap_or(0) == other.view_count.unwrap_or(0)
            && self.last_viewed_at == other.last_viewed_at
    }
}

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
        // `artUrl` is included so incremental sync can detect art-only edits
        // (Plex bumps the timestamp suffix on the thumb path but does not
        // always touch `updatedAt`).
        let mut stmt = conn.prepare(
            "SELECT sourceId, id, updatedAt, firstGenre, firstCollection, artUrl FROM albums",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                CachedAlbumInfo {
                    id: row.get(1)?,
                    updated_at: row.get(2)?,
                    first_genre: row.get(3)?,
                    first_collection: row.get(4)?,
                    art_url: row.get(5)?,
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

    /// Per-user album state (`rating`, `viewCount`, `lastViewedAt`) keyed by
    /// sourceId, for comparison against the album listing.
    pub fn album_user_state(&self) -> Result<HashMap<String, AlbumUserState>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt =
            conn.prepare("SELECT sourceId, rating, viewCount, lastViewedAt FROM albums")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                AlbumUserState {
                    rating: row.get(1)?,
                    view_count: row.get(2)?,
                    last_viewed_at: row.get(3)?,
                },
            ))
        })?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    /// Overwrites per-user album state, `None` included: a favourite cleared
    /// or an album marked unplayed on the server must clear here too.
    pub fn set_album_user_state(
        &self,
        rows: &[(String, AlbumUserState)],
    ) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "UPDATE albums SET rating = ?1, viewCount = ?2, lastViewedAt = ?3 WHERE sourceId = ?4",
            )?;
            for (source_id, s) in rows {
                stmt.execute(params![s.rating, s.view_count, s.last_viewed_at, source_id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Track sourceIds grouped by their album's sourceId.
    pub fn track_ids_by_album(&self) -> Result<HashMap<String, Vec<String>>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT a.sourceId, t.sourceId FROM tracks t JOIN albums a ON a.id = t.albumId",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for row in rows {
            let (album, track) = row?;
            map.entry(album).or_default().push(track);
        }
        Ok(map)
    }

    /// Server time (Unix seconds) from which the next quick sync of this
    /// library asks for changed tracks. `None` until a sync has completed.
    pub fn sync_mark(&self, library_key: &str) -> Result<Option<i64>, CacheError> {
        let conn = self.conn.lock();
        let r = conn.query_row(
            "SELECT changedSince FROM sync_marks WHERE libraryKey = ?1",
            params![library_key],
            |row| row.get(0),
        );
        match r {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_sync_mark(&self, library_key: &str, changed_since: i64) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO sync_marks (libraryKey, changedSince) VALUES (?1, ?2)
             ON CONFLICT(libraryKey) DO UPDATE SET changedSince = excluded.changedSince",
            params![library_key, changed_since],
        )?;
        Ok(())
    }
}
