use std::collections::HashMap;

use rusqlite::params;

use super::db::{CacheDatabase, CacheError};

// --- Upsert row types ---

/// (name, sortName, sourceId, artUrl, summary, updatedAt)
pub type ArtistUpsertRow = (
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
);

/// (id, name, sourceId, artUrl)
pub type ArtistRow = (i64, String, String, Option<String>);

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
    /// First genre in Plex API response order, lowercased. Compared against
    /// the API-order first genre on every incremental sync to detect
    /// genre-only edits (Plex doesn't always bump updatedAt for those).
    /// Must come from the same list call that sets updatedAt — storing a
    /// sorted/alphabetical value here causes every multi-genre album to
    /// look "changed" on every sync.
    pub first_genre: Option<String>,
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

impl CacheDatabase {
    /// Batch upsert artists. Returns sourceId -> local id map for the upserted rows.
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
                     updatedAt = excluded.updatedAt
                 RETURNING id, sourceId",
            )?;

            for (name, sort_name, source_id, art_url, summary, updated_at) in items {
                let row = stmt.query_row(
                    params![name, sort_name, source_id, art_url, summary, updated_at],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                )?;
                map.insert(row.1, row.0);
            }
        }

        tx.commit()?;
        Ok(map)
    }

    /// Batch upsert albums. Returns sourceId -> local id map for the upserted rows.
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
                "INSERT INTO albums (title, artistId, year, sourceId, artUrl, updatedAt, addedAt, lastViewedAt, firstGenre)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(sourceId) DO UPDATE SET
                     title = excluded.title,
                     artistId = excluded.artistId,
                     year = excluded.year,
                     artUrl = excluded.artUrl,
                     rating = COALESCE(excluded.rating, albums.rating),
                     studio = COALESCE(excluded.studio, albums.studio),
                     updatedAt = excluded.updatedAt,
                     addedAt = COALESCE(excluded.addedAt, albums.addedAt),
                     lastViewedAt = COALESCE(excluded.lastViewedAt, albums.lastViewedAt),
                     firstGenre = COALESCE(excluded.firstGenre, albums.firstGenre)
                 RETURNING id, sourceId",
            )?;

            for row in items {
                let result = stmt.query_row(
                    params![
                        row.title,
                        row.artist_id,
                        row.year,
                        row.source_id,
                        row.art_url,
                        row.updated_at,
                        row.added_at,
                        row.last_viewed_at,
                        row.first_genre,
                    ],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
                )?;
                map.insert(result.1, result.0);
            }
        }

        tx.commit()?;
        Ok(map)
    }

    /// Batch upsert tracks. Uses RETURNING to get rowids for FTS5 without extra SELECTs.
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
                     updatedAt = excluded.updatedAt
                 RETURNING id",
            )?;

            let mut fts_stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO tracks_fts(rowid, title) VALUES (?1, ?2)",
            )?;

            for row in items {
                let track_id: i64 = stmt.query_row(
                    params![
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
                    ],
                    |r| r.get(0),
                )?;
                fts_stmt.execute(params![track_id, row.title])?;
            }
        }

        tx.commit()?;
        Ok(())
    }
}
