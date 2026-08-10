use std::collections::HashSet;

use rusqlite::params;

use super::db::{CacheDatabase, CacheError};

impl CacheDatabase {
    /// Batch upsert collections and album ↔ collection links. `items` is `(album_id, collection_names)`.
    pub fn batch_upsert_collections_and_links(
        &self,
        items: &[(i64, Vec<String>)],
    ) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;

        {
            let mut col_stmt = tx.prepare_cached(
                "INSERT INTO collections (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            )?;
            let mut id_stmt =
                tx.prepare_cached("SELECT id FROM collections WHERE name = ?1 COLLATE NOCASE")?;
            let mut link_stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO album_collections (albumId, collectionId) VALUES (?1, ?2)",
            )?;

            for (album_id, collection_names) in items {
                for name in collection_names {
                    col_stmt.execute(params![name])?;
                    let col_id: i64 = id_stmt.query_row(params![name], |r| r.get(0))?;
                    link_stmt.execute(params![album_id, col_id])?;
                }
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Upsert a collection name case-insensitively and return its id.
    pub fn upsert_collection(&self, name: &str) -> Result<i64, CacheError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO collections (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            params![name],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM collections WHERE name = ?1 COLLATE NOCASE",
            params![name],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    /// Link an album to a collection. Duplicate links are ignored.
    pub fn link_album_collection(
        &self,
        album_id: i64,
        collection_id: i64,
    ) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR IGNORE INTO album_collections (albumId, collectionId) VALUES (?1, ?2)",
            params![album_id, collection_id],
        )?;
        Ok(())
    }

    /// Replace all collections for an album.
    pub fn set_album_collections(
        &self,
        album_id: i64,
        collection_ids: &[i64],
    ) -> Result<(), CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM album_collections WHERE albumId = ?1",
            params![album_id],
        )?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO album_collections (albumId, collectionId) VALUES (?1, ?2)",
            )?;
            for cid in collection_ids {
                stmt.execute(params![album_id, cid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Tag an album (by Plex ratingKey) with a collection, creating the
    /// collection row if needed. Returns `false` when no album with that
    /// sourceId exists. Duplicate links are ignored.
    pub fn add_album_to_collection(
        &self,
        source_id: &str,
        name: &str,
    ) -> Result<bool, CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;

        let album_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM albums WHERE sourceId = ?1",
                params![source_id],
                |r| r.get(0),
            )
            .ok();
        let Some(album_id) = album_id else {
            return Ok(false);
        };

        tx.execute(
            "INSERT INTO collections (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            params![name],
        )?;
        let col_id: i64 = tx.query_row(
            "SELECT id FROM collections WHERE name = ?1 COLLATE NOCASE",
            params![name],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO album_collections (albumId, collectionId) VALUES (?1, ?2)",
            params![album_id, col_id],
        )?;

        tx.commit()?;
        Ok(true)
    }

    /// Untag an album (by Plex ratingKey) from a collection. Returns `false`
    /// when the album or collection is unknown, or the link didn't exist.
    /// A collection left with no albums is dropped, mirroring the server
    /// (Plex auto-deletes emptied collections).
    pub fn remove_album_from_collection(
        &self,
        source_id: &str,
        name: &str,
    ) -> Result<bool, CacheError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;

        let removed = tx.execute(
            "DELETE FROM album_collections
             WHERE albumId = (SELECT id FROM albums WHERE sourceId = ?1)
               AND collectionId = (SELECT id FROM collections WHERE name = ?2 COLLATE NOCASE)",
            params![source_id, name],
        )?;
        if removed > 0 {
            tx.execute(
                "DELETE FROM collections
                 WHERE name = ?1 COLLATE NOCASE
                   AND id NOT IN (SELECT DISTINCT collectionId FROM album_collections)",
                params![name],
            )?;
        }

        tx.commit()?;
        Ok(removed > 0)
    }

    /// Collection names for an album.
    pub fn album_collections(&self, source_id: &str) -> Result<Vec<String>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT c.name FROM collections c
             JOIN album_collections ac ON ac.collectionId = c.id
             JOIN albums a ON a.id = ac.albumId
             WHERE a.sourceId = ?1
             ORDER BY c.name COLLATE NOCASE",
        )?;
        let names = stmt
            .query_map(params![source_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(names)
    }

    /// Album IDs tagged with any of the given collection names.
    pub fn album_ids_for_collection_names(
        &self,
        collection_names: &[String],
    ) -> Result<HashSet<i64>, CacheError> {
        if collection_names.is_empty() {
            return Ok(HashSet::new());
        }
        let conn = self.conn.lock();
        let placeholders = collection_names
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT DISTINCT ac.albumId
             FROM album_collections ac
             JOIN collections c ON c.id = ac.collectionId
             WHERE c.name COLLATE NOCASE IN ({})",
            placeholders
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(collection_names.iter()),
            |row| row.get::<_, i64>(0),
        )?;
        let mut set = HashSet::new();
        for row in rows {
            set.insert(row?);
        }
        Ok(set)
    }

    /// All collection names.
    pub fn all_collection_names(&self) -> Result<Vec<String>, CacheError> {
        let conn = self.conn.lock();
        let mut stmt =
            conn.prepare("SELECT name FROM collections ORDER BY name COLLATE NOCASE")?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(names)
    }
}
