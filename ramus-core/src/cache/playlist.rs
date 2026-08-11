//! Local mirror of the server's audio playlists. Written only AFTER a
//! successful server fetch/mutation (same policy as collections); reads
//! serve the Lists surfaces when the server is unreachable.

use rusqlite::params;

use super::db::{CacheDatabase, CacheError};
use crate::models::{Playlist, PlaylistItem};

/// One playlist row as fetched from the server, ready to mirror.
#[derive(Debug, Clone)]
pub struct PlaylistUpsertRow {
    pub source_id: String,
    pub title: String,
    pub smart: bool,
    pub track_count: Option<i64>,
    pub duration_ms: Option<i64>,
    pub thumb: Option<String>,
}

/// One playlist entry as fetched from the server: Plex's per-item id plus
/// the track's ratingKey, in playlist order.
#[derive(Debug, Clone)]
pub struct PlaylistItemRow {
    pub plex_item_id: Option<i64>,
    pub track_source_id: String,
}

impl CacheDatabase {
    /// Mirror the full server-side playlist list: upsert every row, drop
    /// local playlists (and their items) the server no longer has.
    pub fn replace_playlists(&self, rows: &[PlaylistUpsertRow]) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        {
            let existing: Vec<(i64, String)> = {
                let mut stmt = tx.prepare("SELECT id, sourceId FROM playlists")?;
                let list = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                list
            };
            for (id, source_id) in existing {
                if !rows.iter().any(|r| r.source_id == source_id) {
                    tx.execute("DELETE FROM playlist_items WHERE playlistId = ?1", params![id])?;
                    tx.execute("DELETE FROM playlists WHERE id = ?1", params![id])?;
                }
            }
            let mut stmt = tx.prepare_cached(
                "INSERT INTO playlists (sourceId, title, smart, trackCount, durationMs, thumb)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(sourceId) DO UPDATE SET
                     title = excluded.title,
                     smart = excluded.smart,
                     trackCount = excluded.trackCount,
                     durationMs = excluded.durationMs,
                     thumb = excluded.thumb",
            )?;
            for row in rows {
                stmt.execute(params![
                    row.source_id,
                    row.title,
                    row.smart as i64,
                    row.track_count,
                    row.duration_ms,
                    row.thumb,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Upsert a single playlist row (create flow — the full list isn't in
    /// hand, so no pruning).
    pub fn upsert_playlist(&self, row: &PlaylistUpsertRow) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO playlists (sourceId, title, smart, trackCount, durationMs, thumb)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(sourceId) DO UPDATE SET
                 title = excluded.title,
                 smart = excluded.smart,
                 trackCount = excluded.trackCount,
                 durationMs = excluded.durationMs,
                 thumb = excluded.thumb",
            params![
                row.source_id,
                row.title,
                row.smart as i64,
                row.track_count,
                row.duration_ms,
                row.thumb,
            ],
        )?;
        Ok(())
    }

    /// Replace a playlist's mirrored entries with the server's current order.
    pub fn replace_playlist_items(
        &self,
        playlist_source_id: &str,
        items: &[PlaylistItemRow],
    ) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        {
            let playlist_id: i64 = tx.query_row(
                "SELECT id FROM playlists WHERE sourceId = ?1",
                params![playlist_source_id],
                |r| r.get(0),
            )?;
            tx.execute(
                "DELETE FROM playlist_items WHERE playlistId = ?1",
                params![playlist_id],
            )?;
            let mut stmt = tx.prepare_cached(
                "INSERT INTO playlist_items (playlistId, plexItemId, trackSourceId, position)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (position, item) in items.iter().enumerate() {
                stmt.execute(params![
                    playlist_id,
                    item.plex_item_id,
                    item.track_source_id,
                    position as i64,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// All mirrored playlists, alphabetical.
    pub fn all_playlists(&self) -> Result<Vec<Playlist>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT sourceId, title, smart, trackCount, durationMs, thumb
             FROM playlists
             ORDER BY title COLLATE NOCASE",
        )?;
        let playlists = stmt
            .query_map([], |row| {
                Ok(Playlist {
                    source_id: row.get(0)?,
                    title: row.get(1)?,
                    smart: row.get::<_, i64>(2)? != 0,
                    track_count: row.get(3)?,
                    duration: row.get::<_, Option<i64>>(4)?.map(|ms| ms as f64 / 1000.0),
                    thumb: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(playlists)
    }

    /// A playlist's mirrored entries in order, joined against the library
    /// tracks. Entries whose track isn't in the synced library (or whose
    /// per-item id never made it into the mirror) are skipped — they can't
    /// be played or addressed anyway.
    pub fn playlist_items(&self, playlist_source_id: &str) -> Result<Vec<PlaylistItem>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT t.sourceId, t.title, ar.name, t.trackArtist,
                    al.title, al.sourceId, t.trackNumber, t.durationMs,
                    t.codec, t.partKey, al.artUrl, t.userRating, t.bitrate, t.discNumber,
                    t.fileSizeBytes, t.ratingCount, pi.plexItemId
             FROM playlist_items pi
             JOIN playlists p ON p.id = pi.playlistId
             JOIN tracks t ON t.sourceId = pi.trackSourceId
             JOIN albums al ON al.id = t.albumId
             JOIN artists ar ON ar.id = t.artistId
             WHERE p.sourceId = ?1
             ORDER BY pi.position",
        )?;
        let items = stmt
            .query_map(params![playlist_source_id], |row| {
                let track = Self::map_track_row(row)?;
                let plex_item_id: Option<i64> = row.get(16)?;
                Ok(plex_item_id.map(|id| PlaylistItem {
                    playlist_item_id: id,
                    track,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        Ok(items)
    }

    /// Drop one playlist (and its entries) from the mirror.
    pub fn remove_playlist(&self, playlist_source_id: &str) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        {
            let playlist_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM playlists WHERE sourceId = ?1",
                    params![playlist_source_id],
                    |r| r.get(0),
                )
                .ok();
            if let Some(id) = playlist_id {
                tx.execute("DELETE FROM playlist_items WHERE playlistId = ?1", params![id])?;
                tx.execute("DELETE FROM playlists WHERE id = ?1", params![id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> CacheDatabase {
        CacheDatabase::open_in_memory().unwrap()
    }

    fn seed_track(db: &CacheDatabase, source_id: &str, title: &str) {
        let conn = db.conn.lock();
        conn.execute(
            "INSERT OR IGNORE INTO artists (name, sourceId) VALUES ('Artist', 'ar1')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO albums (title, sourceId, artistId)
             VALUES ('Album', 'al1', (SELECT id FROM artists WHERE sourceId = 'ar1'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tracks (title, albumId, artistId, sourceId, durationMs)
             VALUES (?1,
                     (SELECT id FROM albums WHERE sourceId = 'al1'),
                     (SELECT id FROM artists WHERE sourceId = 'ar1'),
                     ?2, 1000)",
            params![title, source_id],
        )
        .unwrap();
    }

    fn row(source_id: &str, title: &str, smart: bool) -> PlaylistUpsertRow {
        PlaylistUpsertRow {
            source_id: source_id.into(),
            title: title.into(),
            smart,
            track_count: Some(2),
            duration_ms: Some(360_000),
            thumb: None,
        }
    }

    #[test]
    fn test_replace_playlists_upserts_and_prunes() {
        let db = setup();
        db.replace_playlists(&[row("p1", "Road Trip", false), row("p2", "Focus", true)])
            .unwrap();
        assert_eq!(db.all_playlists().unwrap().len(), 2);

        // p2 disappears server-side; p1 renamed.
        db.replace_playlists(&[row("p1", "Road Trip 2", false)]).unwrap();
        let playlists = db.all_playlists().unwrap();
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].title, "Road Trip 2");
        assert!(!playlists[0].smart);
        assert_eq!(playlists[0].duration, Some(360.0));
    }

    #[test]
    fn test_playlist_items_join_order_and_skips() {
        let db = setup();
        seed_track(&db, "t1", "One");
        seed_track(&db, "t2", "Two");
        db.replace_playlists(&[row("p1", "Mix", false)]).unwrap();
        db.replace_playlist_items(
            "p1",
            &[
                PlaylistItemRow { plex_item_id: Some(11), track_source_id: "t2".into() },
                PlaylistItemRow { plex_item_id: Some(12), track_source_id: "t1".into() },
                // Not in the library — must be skipped, not error.
                PlaylistItemRow { plex_item_id: Some(13), track_source_id: "missing".into() },
                // No per-item id — unaddressable, skipped.
                PlaylistItemRow { plex_item_id: None, track_source_id: "t1".into() },
            ],
        )
        .unwrap();

        let items = db.playlist_items("p1").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].playlist_item_id, 11);
        assert_eq!(items[0].track.title, "Two");
        assert_eq!(items[1].playlist_item_id, 12);
        assert_eq!(items[1].track.title, "One");
    }

    #[test]
    fn test_replace_playlist_items_replaces_wholesale() {
        let db = setup();
        seed_track(&db, "t1", "One");
        db.replace_playlists(&[row("p1", "Mix", false)]).unwrap();
        db.replace_playlist_items(
            "p1",
            &[PlaylistItemRow { plex_item_id: Some(1), track_source_id: "t1".into() }],
        )
        .unwrap();
        db.replace_playlist_items(
            "p1",
            &[PlaylistItemRow { plex_item_id: Some(2), track_source_id: "t1".into() }],
        )
        .unwrap();
        let items = db.playlist_items("p1").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].playlist_item_id, 2);
    }

    #[test]
    fn test_remove_playlist_drops_items() {
        let db = setup();
        seed_track(&db, "t1", "One");
        db.replace_playlists(&[row("p1", "Mix", false)]).unwrap();
        db.replace_playlist_items(
            "p1",
            &[PlaylistItemRow { plex_item_id: Some(1), track_source_id: "t1".into() }],
        )
        .unwrap();
        db.remove_playlist("p1").unwrap();
        assert!(db.all_playlists().unwrap().is_empty());
        let orphans: i64 = db
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM playlist_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(orphans, 0);
    }
}
