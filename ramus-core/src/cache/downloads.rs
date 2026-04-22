//! Persistent download CRUD on the `downloads` table.
//!
//! Downloads are user-requested, never evicted, and live at
//! `config_dir()/downloads/<ratingKey>.<ext>`. The table records what's
//! on disk so the player can rehydrate its persistent cache at startup
//! and the UI can display storage usage without scanning the filesystem.

use std::collections::{HashMap, HashSet};

use rusqlite::params;

use super::db::{CacheDatabase, CacheError};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRow {
    pub rating_key: String,
    pub album_rating_key: String,
    pub file_path: String,
    pub size_bytes: i64,
    pub codec: String,
    pub downloaded_at: i64,
}

impl CacheDatabase {
    /// Insert or replace a download record.
    pub fn insert_download(&self, row: &DownloadRow) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO downloads (ratingKey, albumRatingKey, filePath, sizeBytes, codec, downloadedAt)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(ratingKey) DO UPDATE SET
                 albumRatingKey = excluded.albumRatingKey,
                 filePath = excluded.filePath,
                 sizeBytes = excluded.sizeBytes,
                 codec = excluded.codec,
                 downloadedAt = excluded.downloadedAt",
            params![
                row.rating_key,
                row.album_rating_key,
                row.file_path,
                row.size_bytes,
                row.codec,
                row.downloaded_at,
            ],
        )?;
        Ok(())
    }

    /// Remove a download record. Returns the file path if one was present.
    pub fn remove_download(&self, rating_key: &str) -> Result<Option<String>, CacheError> {
        let conn = self.conn.lock();
        let path: Option<String> = conn
            .query_row(
                "SELECT filePath FROM downloads WHERE ratingKey = ?1",
                params![rating_key],
                |r| r.get(0),
            )
            .ok();
        conn.execute("DELETE FROM downloads WHERE ratingKey = ?1", params![rating_key])?;
        Ok(path)
    }

    /// Remove all downloads for an album. Returns the file paths that were removed.
    pub fn remove_album_downloads(&self, album_rating_key: &str) -> Result<Vec<String>, CacheError> {
        let conn = self.conn.lock();
        let paths: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT filePath FROM downloads WHERE albumRatingKey = ?1",
            )?;
            let rows = stmt
                .query_map(params![album_rating_key], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        conn.execute(
            "DELETE FROM downloads WHERE albumRatingKey = ?1",
            params![album_rating_key],
        )?;
        Ok(paths)
    }

    /// Remove all downloads. Returns every file path for disk cleanup.
    pub fn clear_all_downloads(&self) -> Result<Vec<String>, CacheError> {
        let conn = self.conn.lock();
        let paths: Vec<String> = {
            let mut stmt = conn.prepare("SELECT filePath FROM downloads")?;
            let rows = stmt
                .query_map([], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        conn.execute("DELETE FROM downloads", [])?;
        Ok(paths)
    }

    /// Returns `(rating_key, file_path)` for every download on disk.
    pub fn all_download_paths(&self) -> Result<Vec<(String, String)>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT ratingKey, filePath FROM downloads")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Returns every downloaded rating key.
    pub fn downloaded_rating_keys(&self) -> Result<HashSet<String>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT ratingKey FROM downloads")?;
        let keys: HashSet<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?;
        Ok(keys)
    }

    /// Set of album rating keys that have at least one track downloaded.
    /// Used by the offline-mode library filter.
    pub fn downloaded_album_source_ids(&self) -> Result<HashSet<String>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT DISTINCT albumRatingKey FROM downloads")?;
        let ids: HashSet<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?;
        Ok(ids)
    }

    /// Internal album rowids that have at least one downloaded track.
    /// Used to intersect with the genre-tree album sets (which key on
    /// internal ids) without an N-round trip.
    pub fn downloaded_album_internal_ids(&self) -> Result<HashSet<i64>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT a.id FROM albums a
             JOIN downloads d ON d.albumRatingKey = a.sourceId",
        )?;
        let ids: HashSet<i64> = stmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<Result<_, _>>()?;
        Ok(ids)
    }

    /// Artist names that have at least one downloaded track. Used by
    /// offline-mode to filter the artists list.
    pub fn downloaded_artist_names(&self) -> Result<HashSet<String>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT ar.name FROM artists ar
             JOIN albums a ON a.artistId = ar.id
             JOIN downloads d ON d.albumRatingKey = a.sourceId",
        )?;
        let names: HashSet<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?;
        Ok(names)
    }

    /// Total size of all downloads in bytes.
    pub fn total_download_bytes(&self) -> Result<i64, CacheError> {
        let conn = self.conn.lock();
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(sizeBytes), 0) FROM downloads",
            [],
            |r| r.get(0),
        )?;
        Ok(total)
    }

    /// Downloaded track count and total size, aggregated per album.
    /// Albums with no downloaded tracks are omitted.
    pub fn downloaded_counts_by_album(
        &self,
    ) -> Result<HashMap<String, (u32, i64)>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT albumRatingKey, COUNT(*), COALESCE(SUM(sizeBytes), 0)
             FROM downloads GROUP BY albumRatingKey",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)? as u32,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().map(|(k, n, s)| (k, (n, s))).collect())
    }

    /// Bulk lookup of total track counts for a set of album rating keys.
    /// Returns a map so the caller avoids N+1 queries when rendering the
    /// downloads panel's album list.
    pub fn album_total_track_counts(
        &self,
        album_rating_keys: &[String],
    ) -> Result<HashMap<String, u32>, CacheError> {
        if album_rating_keys.is_empty() {
            return Ok(HashMap::new());
        }
        let conn = self.conn.lock();
        const CHUNK_SIZE: usize = 500;
        let mut out = HashMap::new();
        for chunk in album_rating_keys.chunks(CHUNK_SIZE) {
            let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT a.sourceId, COUNT(t.id)
                 FROM albums a
                 LEFT JOIN tracks t ON t.albumId = a.id
                 WHERE a.sourceId IN ({})
                 GROUP BY a.sourceId",
                placeholders
            );
            let mut stmt = conn.prepare(&sql)?;
            let params: Vec<&dyn rusqlite::types::ToSql> = chunk
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = stmt.query_map(params.as_slice(), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32))
            })?;
            for row in rows {
                let (k, n) = row?;
                out.insert(k, n);
            }
        }
        Ok(out)
    }

    /// List downloaded tracks that belong to albums with zero other downloaded
    /// tracks — "orphan" individual tracks for the downloads panel's
    /// "Individual Tracks" section.
    pub fn orphan_downloaded_tracks(&self) -> Result<Vec<DownloadRow>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT d.ratingKey, d.albumRatingKey, d.filePath, d.sizeBytes, d.codec, d.downloadedAt
             FROM downloads d
             WHERE d.albumRatingKey IN (
                 SELECT albumRatingKey FROM downloads GROUP BY albumRatingKey HAVING COUNT(*) = 1
             )",
        )?;
        let rows = stmt
            .query_map([], map_download_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

fn map_download_row(row: &rusqlite::Row) -> rusqlite::Result<DownloadRow> {
    Ok(DownloadRow {
        rating_key: row.get(0)?,
        album_rating_key: row.get(1)?,
        file_path: row.get(2)?,
        size_bytes: row.get(3)?,
        codec: row.get(4)?,
        downloaded_at: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::upsert::{AlbumUpsertRow, TrackUpsertRow};

    fn setup() -> CacheDatabase {
        let db = CacheDatabase::open_in_memory().unwrap();
        let artist_map = db
            .batch_upsert_artists(&[(
                "Radiohead".into(),
                None,
                "ar1".into(),
                None,
                None,
                None,
                Some(1000),
            )])
            .unwrap();
        let artist_id = *artist_map.get("ar1").unwrap();
        let album_map = db
            .batch_upsert_albums(&[AlbumUpsertRow {
                title: "OK Computer".into(),
                artist_id,
                year: Some(1997),
                source_id: "al1".into(),
                art_url: None,
                updated_at: Some(1000),
                added_at: None,
                last_viewed_at: None,
                first_genre: None,
                first_collection: None,
                view_count: None,
            }])
            .unwrap();
        let album_id = *album_map.get("al1").unwrap();
        db.batch_upsert_tracks(&[
            TrackUpsertRow {
                title: "Paranoid Android".into(),
                album_id,
                artist_id,
                track_number: Some(1),
                disc_number: Some(1),
                duration_ms: Some(384000),
                source_id: "tr1".into(),
                codec: Some("flac".into()),
                part_key: None,
                stream_id: None,
                user_rating: None,
                bitrate: Some(900),
                track_artist: None,
                updated_at: Some(1000),
                file_size_bytes: Some(42_000_000),
                rating_count: None,
            },
            TrackUpsertRow {
                title: "Karma Police".into(),
                album_id,
                artist_id,
                track_number: Some(2),
                disc_number: Some(1),
                duration_ms: Some(264000),
                source_id: "tr2".into(),
                codec: Some("flac".into()),
                part_key: None,
                stream_id: None,
                user_rating: None,
                bitrate: Some(900),
                track_artist: None,
                updated_at: Some(1000),
                file_size_bytes: Some(30_000_000),
                rating_count: None,
            },
        ])
        .unwrap();
        db
    }

    fn dl(rating_key: &str, album: &str, size: i64) -> DownloadRow {
        DownloadRow {
            rating_key: rating_key.into(),
            album_rating_key: album.into(),
            file_path: format!("/tmp/{rating_key}.flac"),
            size_bytes: size,
            codec: "flac".into(),
            downloaded_at: 12345,
        }
    }

    #[test]
    fn test_insert_and_list() {
        let db = setup();
        db.insert_download(&dl("tr1", "al1", 100)).unwrap();
        db.insert_download(&dl("tr2", "al1", 200)).unwrap();

        let paths = db.all_download_paths().unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(db.total_download_bytes().unwrap(), 300);

        let albums = db.downloaded_album_source_ids().unwrap();
        assert!(albums.contains("al1"));
    }

    #[test]
    fn test_insert_replaces_existing() {
        let db = setup();
        db.insert_download(&dl("tr1", "al1", 100)).unwrap();
        db.insert_download(&dl("tr1", "al1", 150)).unwrap();
        assert_eq!(db.total_download_bytes().unwrap(), 150);
    }

    #[test]
    fn test_remove_returns_path() {
        let db = setup();
        db.insert_download(&dl("tr1", "al1", 100)).unwrap();
        let path = db.remove_download("tr1").unwrap();
        assert_eq!(path, Some("/tmp/tr1.flac".into()));
        assert!(db.remove_download("tr1").unwrap().is_none());
    }

    #[test]
    fn test_remove_album_downloads() {
        let db = setup();
        db.insert_download(&dl("tr1", "al1", 100)).unwrap();
        db.insert_download(&dl("tr2", "al1", 200)).unwrap();
        let paths = db.remove_album_downloads("al1").unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(db.total_download_bytes().unwrap(), 0);
    }

    #[test]
    fn test_clear_all_downloads() {
        let db = setup();
        db.insert_download(&dl("tr1", "al1", 100)).unwrap();
        db.insert_download(&dl("tr2", "al1", 200)).unwrap();
        let paths = db.clear_all_downloads().unwrap();
        assert_eq!(paths.len(), 2);
        assert!(db.downloaded_rating_keys().unwrap().is_empty());
    }

    #[test]
    fn test_downloaded_counts_by_album() {
        let db = setup();
        db.insert_download(&dl("tr1", "al1", 100)).unwrap();
        db.insert_download(&dl("tr2", "al1", 200)).unwrap();
        let counts = db.downloaded_counts_by_album().unwrap();
        let (n, sz) = counts.get("al1").copied().unwrap();
        assert_eq!(n, 2);
        assert_eq!(sz, 300);
    }

    #[test]
    fn test_album_total_track_counts_bulk() {
        let db = setup();
        let totals = db
            .album_total_track_counts(&["al1".into(), "al_missing".into()])
            .unwrap();
        assert_eq!(totals.get("al1").copied(), Some(2));
        assert_eq!(totals.get("al_missing"), None);
    }

    #[test]
    fn test_orphan_downloaded_tracks() {
        let db = setup();
        db.insert_download(&dl("tr1", "al1", 100)).unwrap();
        let orphans = db.orphan_downloaded_tracks().unwrap();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].rating_key, "tr1");

        db.insert_download(&dl("tr2", "al1", 200)).unwrap();
        let orphans = db.orphan_downloaded_tracks().unwrap();
        assert_eq!(orphans.len(), 0, "album with multiple downloads has no orphans");
    }
}
