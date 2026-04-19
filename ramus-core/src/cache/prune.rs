use std::collections::HashSet;

use rusqlite::params;

use super::db::{CacheDatabase, CacheError};

/// Counts of items removed during pruning.
#[derive(Debug, Clone, Default)]
pub struct PruneCounts {
    pub artists: usize,
    pub albums: usize,
    pub tracks: usize,
}

impl PruneCounts {
    pub fn total(&self) -> usize {
        self.artists + self.albums + self.tracks
    }
}

const CHUNK_SIZE: usize = 500;

impl CacheDatabase {
    /// Remove local items whose sourceIds are no longer present on the Plex server.
    /// Deletes in leaf-first order (tracks → albums → artists) and cleans up FTS5.
    /// Foreign-key cascades handle stragglers: deleting an album cascades to its
    /// tracks and `album_genres` rows.
    pub fn prune_removed(
        &self,
        plex_artist_ids: &HashSet<String>,
        plex_album_ids: &HashSet<String>,
        plex_track_ids: &HashSet<String>,
    ) -> Result<PruneCounts, CacheError> {
        let conn = self.conn.lock();

        let db_artist_ids = Self::all_source_ids(&conn, "artists")?;
        let db_album_ids = Self::all_source_ids(&conn, "albums")?;
        let db_track_ids = Self::all_source_ids(&conn, "tracks")?;

        // Items in the DB but not on Plex.
        let stale_tracks: Vec<&str> = db_track_ids
            .iter()
            .filter(|id| !plex_track_ids.contains(*id))
            .map(String::as_str)
            .collect();
        let stale_albums: Vec<&str> = db_album_ids
            .iter()
            .filter(|id| !plex_album_ids.contains(*id))
            .map(String::as_str)
            .collect();
        let stale_artists: Vec<&str> = db_artist_ids
            .iter()
            .filter(|id| !plex_artist_ids.contains(*id))
            .map(String::as_str)
            .collect();

        let counts = PruneCounts {
            tracks: stale_tracks.len(),
            albums: stale_albums.len(),
            artists: stale_artists.len(),
        };

        if counts.total() == 0 {
            return Ok(counts);
        }

        let tx = conn.unchecked_transaction()?;

        // FK cascades do not touch the external-content FTS5 table, so any
        // track that disappears — explicitly stale or cascade-deleted — needs
        // an explicit FTS5 delete first, or zombie entries bloat the index.
        Self::delete_tracks_fts_for_stale(&tx, &stale_tracks, &stale_albums, &stale_artists)?;

        for chunk in stale_tracks.chunks(CHUNK_SIZE) {
            Self::delete_by_source_ids(&tx, "tracks", chunk)?;
        }

        // Albums cascade to child tracks and album_genres.
        for chunk in stale_albums.chunks(CHUNK_SIZE) {
            Self::delete_by_source_ids(&tx, "albums", chunk)?;
        }

        // Artists cascade through albums → tracks → album_genres.
        for chunk in stale_artists.chunks(CHUNK_SIZE) {
            Self::delete_by_source_ids(&tx, "artists", chunk)?;
        }

        // Orphaned genres.
        tx.execute_batch(
            "DELETE FROM genres WHERE id NOT IN (SELECT DISTINCT genreId FROM album_genres)",
        )?;

        tx.commit()?;
        Ok(counts)
    }

    fn all_source_ids(
        conn: &rusqlite::Connection,
        table: &str,
    ) -> Result<HashSet<String>, CacheError> {
        let sql = format!("SELECT sourceId FROM {table}");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut set = HashSet::new();
        for row in rows {
            set.insert(row?);
        }
        Ok(set)
    }

    /// Issue FTS5 delete commands for every track that will disappear —
    /// explicitly stale rows plus those cascade-deleted through their parent
    /// album or artist.
    fn delete_tracks_fts_for_stale(
        tx: &rusqlite::Transaction,
        stale_tracks: &[&str],
        stale_albums: &[&str],
        stale_artists: &[&str],
    ) -> Result<(), CacheError> {
        let mut all_tracks: Vec<(i64, String)> = Vec::new();

        // Explicitly stale tracks.
        for chunk in stale_tracks.chunks(CHUNK_SIZE) {
            let ph = Self::placeholders(chunk.len());
            let sql = format!("SELECT id, title FROM tracks WHERE sourceId IN ({ph})");
            let params = Self::to_sql_params(chunk);
            let mut stmt = tx.prepare(&sql)?;
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            all_tracks.extend(rows);
        }

        // Tracks whose parent album is stale.
        for chunk in stale_albums.chunks(CHUNK_SIZE) {
            let ph = Self::placeholders(chunk.len());
            let sql = format!(
                "SELECT t.id, t.title FROM tracks t
                 JOIN albums a ON a.id = t.albumId
                 WHERE a.sourceId IN ({ph})"
            );
            let params = Self::to_sql_params(chunk);
            let mut stmt = tx.prepare(&sql)?;
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            all_tracks.extend(rows);
        }

        // Tracks whose parent artist is stale.
        for chunk in stale_artists.chunks(CHUNK_SIZE) {
            let ph = Self::placeholders(chunk.len());
            let sql = format!(
                "SELECT t.id, t.title FROM tracks t
                 JOIN artists ar ON ar.id = t.artistId
                 WHERE ar.sourceId IN ({ph})"
            );
            let params = Self::to_sql_params(chunk);
            let mut stmt = tx.prepare(&sql)?;
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            all_tracks.extend(rows);
        }

        // A track may appear in more than one set.
        all_tracks.sort_unstable_by_key(|(id, _)| *id);
        all_tracks.dedup_by_key(|(id, _)| *id);

        let mut fts_del = tx.prepare_cached(
            "INSERT INTO tracks_fts(tracks_fts, rowid, title) VALUES('delete', ?1, ?2)",
        )?;
        for (id, title) in &all_tracks {
            fts_del.execute(params![id, title])?;
        }
        Ok(())
    }

    fn placeholders(n: usize) -> String {
        (1..=n).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ")
    }

    fn to_sql_params<'a>(source_ids: &'a [&'a str]) -> Vec<&'a dyn rusqlite::types::ToSql> {
        source_ids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect()
    }

    fn delete_by_source_ids(
        tx: &rusqlite::Transaction,
        table: &str,
        source_ids: &[&str],
    ) -> Result<(), CacheError> {
        let placeholders: String = (1..=source_ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("DELETE FROM {table} WHERE sourceId IN ({placeholders})");
        let mut stmt = tx.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = source_ids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        stmt.execute(params.as_slice())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db::CacheDatabase;
    use crate::cache::upsert::{AlbumUpsertRow, TrackUpsertRow};

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

    fn seed_album(db: &CacheDatabase, source_id: &str, title: &str, artist_id: i64) -> i64 {
        let map = db
            .batch_upsert_albums(&[AlbumUpsertRow {
                title: title.into(),
                artist_id,
                year: Some(2000),
                source_id: source_id.into(),
                art_url: None,
                updated_at: Some(1000),
                added_at: None,
                last_viewed_at: None,
                first_genre: None,
                first_collection: None,
            }])
            .unwrap();
        *map.get(source_id).unwrap()
    }

    fn seed_track(db: &CacheDatabase, source_id: &str, title: &str, album_id: i64, artist_id: i64) {
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
            file_size_bytes: None,
        }])
        .unwrap();
    }

    fn id_set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_prune_nothing_when_all_present() {
        let db = setup();
        let ar = seed_artist(&db, "ar1", "Radiohead");
        let al = seed_album(&db, "al1", "OK Computer", ar);
        seed_track(&db, "tr1", "Paranoid Android", al, ar);

        let counts = db
            .prune_removed(
                &id_set(&["ar1"]),
                &id_set(&["al1"]),
                &id_set(&["tr1"]),
            )
            .unwrap();

        assert_eq!(counts.total(), 0);
        let stats = db.cache_stats().unwrap();
        assert_eq!(stats.artist_count, 1);
        assert_eq!(stats.album_count, 1);
        assert_eq!(stats.track_count, 1);
    }

    #[test]
    fn test_prune_removed_track() {
        let db = setup();
        let ar = seed_artist(&db, "ar1", "Radiohead");
        let al = seed_album(&db, "al1", "OK Computer", ar);
        seed_track(&db, "tr1", "Paranoid Android", al, ar);
        seed_track(&db, "tr2", "Karma Police", al, ar);

        let counts = db
            .prune_removed(
                &id_set(&["ar1"]),
                &id_set(&["al1"]),
                &id_set(&["tr1"]),
            )
            .unwrap();

        assert_eq!(counts.tracks, 1);
        assert_eq!(counts.albums, 0);
        assert_eq!(counts.artists, 0);

        let stats = db.cache_stats().unwrap();
        assert_eq!(stats.track_count, 1);

        // FTS5 should still work for the remaining track
        let results = db.search_tracks_fts("paranoid").unwrap();
        assert_eq!(results.len(), 1);
        // Pruned track shouldn't appear in FTS
        let results = db.search_tracks_fts("karma").unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_prune_removed_album_cascades_tracks() {
        let db = setup();
        let ar = seed_artist(&db, "ar1", "Radiohead");
        let al1 = seed_album(&db, "al1", "OK Computer", ar);
        let al2 = seed_album(&db, "al2", "Kid A", ar);
        seed_track(&db, "tr1", "Paranoid Android", al1, ar);
        seed_track(&db, "tr2", "Everything In Its Right Place", al2, ar);

        let counts = db
            .prune_removed(
                &id_set(&["ar1"]),
                &id_set(&["al1"]),
                &id_set(&["tr1"]),
            )
            .unwrap();

        assert_eq!(counts.albums, 1);
        assert_eq!(counts.tracks, 1);

        let stats = db.cache_stats().unwrap();
        assert_eq!(stats.album_count, 1);
        assert_eq!(stats.track_count, 1);
    }

    #[test]
    fn test_prune_removed_artist_cascades_all() {
        let db = setup();
        let ar1 = seed_artist(&db, "ar1", "Radiohead");
        let ar2 = seed_artist(&db, "ar2", "Bjork");
        let al1 = seed_album(&db, "al1", "OK Computer", ar1);
        let al2 = seed_album(&db, "al2", "Homogenic", ar2);
        seed_track(&db, "tr1", "Paranoid Android", al1, ar1);
        seed_track(&db, "tr2", "Joga", al2, ar2);

        let genre_id = db.upsert_genre("Rock").unwrap();
        db.link_album_genre(al1, genre_id).unwrap();
        let genre_id2 = db.upsert_genre("Electronic").unwrap();
        db.link_album_genre(al2, genre_id2).unwrap();

        let counts = db
            .prune_removed(
                &id_set(&["ar1"]),
                &id_set(&["al1"]),
                &id_set(&["tr1"]),
            )
            .unwrap();

        assert_eq!(counts.artists, 1);
        assert_eq!(counts.albums, 1);
        assert_eq!(counts.tracks, 1);

        let stats = db.cache_stats().unwrap();
        assert_eq!(stats.artist_count, 1);
        assert_eq!(stats.album_count, 1);
        assert_eq!(stats.track_count, 1);
        // "Electronic" is orphaned and pruned; "Rock" remains.
        assert_eq!(stats.genre_count, 1);
    }

    #[test]
    fn test_prune_with_empty_plex_clears_everything() {
        let db = setup();
        let ar = seed_artist(&db, "ar1", "Radiohead");
        let al = seed_album(&db, "al1", "OK Computer", ar);
        seed_track(&db, "tr1", "Paranoid Android", al, ar);

        let counts = db
            .prune_removed(
                &HashSet::new(),
                &HashSet::new(),
                &HashSet::new(),
            )
            .unwrap();

        assert_eq!(counts.artists, 1);
        assert_eq!(counts.albums, 1);
        assert_eq!(counts.tracks, 1);

        let stats = db.cache_stats().unwrap();
        assert_eq!(stats.artist_count, 0);
        assert_eq!(stats.album_count, 0);
        assert_eq!(stats.track_count, 0);
    }
}
