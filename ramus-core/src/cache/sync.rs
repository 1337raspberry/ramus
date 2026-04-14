use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::cache::db::{CacheDatabase, CacheError};
use crate::cache::upsert::{AlbumUpsertRow, TrackUpsertRow};
use crate::plex::client::{MediaItem, PlexClient, PlexClientError};

// --- Progress ---

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncPhase {
    Artists,
    Albums,
    Tracks,
    DeepGenres,
    Done,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    pub phase: SyncPhase,
    pub current: usize,
    pub total: usize,
    pub detail: String,
}

impl SyncProgress {
    pub fn fraction(&self) -> f64 {
        if self.total > 0 {
            self.current as f64 / self.total as f64
        } else {
            0.0
        }
    }
}

// --- Errors ---

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("plex error: {0}")]
    Plex(#[from] PlexClientError),
    #[error("cache error: {0}")]
    Cache(#[from] CacheError),
}

// --- SyncEngine ---

const BATCH_SIZE: usize = 500;
const DEEP_GENRE_CONCURRENCY: usize = 8;

pub struct SyncEngine {
    pub cache: Arc<CacheDatabase>,
    pub client: Arc<PlexClient>,
}

impl SyncEngine {
    pub fn new(cache: Arc<CacheDatabase>, client: Arc<PlexClient>) -> Self {
        Self { cache, client }
    }

    /// Full sync: all artists/albums/tracks + deep genre fetch for ALL albums.
    pub async fn full_sync<F>(
        &self,
        library_key: &str,
        on_progress: F,
    ) -> Result<(), SyncError>
    where
        F: Fn(SyncProgress) + Send + Sync + 'static,
    {
        let on_progress = Arc::new(on_progress);
        let (album_map, _) =
            self.core_sync(library_key, false, on_progress.clone()).await?;

        // Phase 4: deep genre sync for ALL albums
        self.deep_genre_sync(&album_map, None, on_progress.clone())
            .await?;

        on_progress(SyncProgress {
            phase: SyncPhase::Done,
            current: 1,
            total: 1,
            detail: "Sync complete".into(),
        });
        Ok(())
    }

    /// Incremental sync: only upserts changed items, deep-fetches changed/new albums.
    pub async fn incremental_sync<F>(
        &self,
        library_key: &str,
        on_progress: F,
    ) -> Result<(), SyncError>
    where
        F: Fn(SyncProgress) + Send + Sync + 'static,
    {
        let on_progress = Arc::new(on_progress);
        let (album_map, changed_source_ids) =
            self.core_sync(library_key, true, on_progress.clone()).await?;

        if !changed_source_ids.is_empty() {
            self.deep_genre_sync(
                &album_map,
                Some(&changed_source_ids),
                on_progress.clone(),
            )
            .await?;
        }

        on_progress(SyncProgress {
            phase: SyncPhase::Done,
            current: 1,
            total: 1,
            detail: "Sync complete".into(),
        });
        Ok(())
    }

    /// Genre sync only: re-fetches full metadata for ALL albums to get complete genre lists.
    pub async fn genre_sync<F>(
        &self,
        on_progress: F,
    ) -> Result<(), SyncError>
    where
        F: Fn(SyncProgress) + Send + Sync + 'static,
    {
        let on_progress = Arc::new(on_progress);
        let cached = self.cache.all_album_timestamps()?;
        let album_map: HashMap<String, i64> =
            cached.into_iter().map(|(k, v)| (k, v.id)).collect();

        self.deep_genre_sync(&album_map, None, on_progress.clone())
            .await?;

        on_progress(SyncProgress {
            phase: SyncPhase::Done,
            current: 1,
            total: 1,
            detail: "Sync complete".into(),
        });
        Ok(())
    }

    // --- Core sync (Phases 1-3) ---

    async fn core_sync(
        &self,
        library_key: &str,
        incremental: bool,
        on_progress: Arc<dyn Fn(SyncProgress) + Send + Sync>,
    ) -> Result<(HashMap<String, i64>, HashSet<String>), SyncError> {
        // Pre-load cached timestamps for incremental mode
        let cached_artists;
        let cached_albums;
        let cached_tracks;

        if incremental {
            cached_artists = self.cache.all_artist_timestamps().unwrap_or_default();
            cached_albums = self.cache.all_album_timestamps().unwrap_or_default();
            cached_tracks = self.cache.all_track_timestamps().unwrap_or_default();
        } else {
            cached_artists = HashMap::new();
            cached_albums = HashMap::new();
            cached_tracks = HashMap::new();
        }

        // Phase 1: Artists
        on_progress(SyncProgress {
            phase: SyncPhase::Artists,
            current: 0,
            total: 0,
            detail: "Fetching artists...".into(),
        });
        let artist_items = self
            .client
            .fetch_all_items(library_key, 8, 200)
            .await?;
        let artist_map =
            self.sync_artists(&artist_items, &cached_artists, incremental, on_progress.as_ref())?;

        // Phase 2: Albums
        on_progress(SyncProgress {
            phase: SyncPhase::Albums,
            current: 0,
            total: 0,
            detail: "Fetching albums...".into(),
        });
        let album_items = self
            .client
            .fetch_all_items(library_key, 9, 200)
            .await?;
        let (album_map, changed_source_ids) = self.sync_albums(
            &album_items,
            &artist_map,
            &cached_albums,
            incremental,
            on_progress.as_ref(),
        )?;

        // Phase 3: Tracks
        on_progress(SyncProgress {
            phase: SyncPhase::Tracks,
            current: 0,
            total: 0,
            detail: "Fetching tracks...".into(),
        });
        let track_items = self
            .client
            .fetch_all_items(library_key, 10, 200)
            .await?;
        self.sync_tracks(
            &track_items,
            &album_map,
            &artist_map,
            &cached_tracks,
            incremental,
            on_progress.as_ref(),
        )?;

        // Prune: remove local items that no longer exist on Plex.
        // Both full and incremental fetched ALL items, so we have the
        // complete "live" set of sourceIds for each type.
        let plex_artist_ids: HashSet<String> =
            artist_items.iter().map(|i| i.rating_key.clone()).collect();
        let plex_album_ids: HashSet<String> =
            album_items.iter().map(|i| i.rating_key.clone()).collect();
        let plex_track_ids: HashSet<String> =
            track_items.iter().map(|i| i.rating_key.clone()).collect();

        let prune_counts =
            self.cache
                .prune_removed(&plex_artist_ids, &plex_album_ids, &plex_track_ids)?;

        if prune_counts.total() > 0 {
            on_progress(SyncProgress {
                phase: SyncPhase::Done,
                current: 0,
                total: 1,
                detail: format!(
                    "Pruned {} removed items ({} artists, {} albums, {} tracks)",
                    prune_counts.total(),
                    prune_counts.artists,
                    prune_counts.albums,
                    prune_counts.tracks,
                ),
            });
        }

        Ok((album_map, changed_source_ids))
    }

    // --- Phase 1: Artists ---

    fn sync_artists(
        &self,
        items: &[MediaItem],
        cached: &HashMap<String, crate::cache::db::CachedItemInfo>,
        incremental: bool,
        on_progress: &dyn Fn(SyncProgress),
    ) -> Result<HashMap<String, i64>, SyncError> {
        let mut map: HashMap<String, i64> = HashMap::new();

        // Pre-seed from cache for incremental
        if incremental {
            for (source_id, info) in cached {
                map.insert(source_id.clone(), info.id);
            }
        }

        // Collect changed items
        type ArtistTuple = (String, Option<String>, String, Option<String>, Option<String>, Option<i64>);
        let mut changed: Vec<ArtistTuple> = Vec::new();

        for item in items {
            if incremental {
                if let Some(info) = cached.get(&item.rating_key) {
                    if info.updated_at == item.updated_at {
                        continue;
                    }
                }
            }
            changed.push((
                item.title.clone(),
                item.title_sort.clone(),
                item.rating_key.clone(),
                item.thumb.clone(),
                item.summary.clone(),
                item.updated_at,
            ));
        }

        // Batch upsert
        let total = changed.len();
        for start in (0..total).step_by(BATCH_SIZE) {
            let end = (start + BATCH_SIZE).min(total);
            let chunk = &changed[start..end];
            let ids = self.cache.batch_upsert_artists(chunk)?;
            map.extend(ids);
            on_progress(SyncProgress {
                phase: SyncPhase::Artists,
                current: end,
                total,
                detail: format!("Artists: {}/{}", end, total),
            });
        }

        Ok(map)
    }

    // --- Phase 2: Albums ---

    fn sync_albums(
        &self,
        items: &[MediaItem],
        artist_map: &HashMap<String, i64>,
        cached: &HashMap<String, crate::cache::db::CachedAlbumInfo>,
        incremental: bool,
        on_progress: &dyn Fn(SyncProgress),
    ) -> Result<(HashMap<String, i64>, HashSet<String>), SyncError> {
        let mut map: HashMap<String, i64> = HashMap::new();
        let mut changed_ids: HashSet<String> = HashSet::new();

        // Pre-seed from cache
        if incremental {
            for (source_id, info) in cached {
                map.insert(source_id.clone(), info.id);
            }
        }

        let mut changed: Vec<AlbumUpsertRow> = Vec::new();
        let mut genre_links: Vec<(String, String)> = Vec::new(); // (album_source_id, genre_name)

        for item in items {
            let artist_key = item.parent_rating_key.as_deref().unwrap_or("");
            let artist_id = match artist_map.get(artist_key) {
                Some(&id) => id,
                None => match self.cache.artist_id(artist_key)? {
                    Some(id) => id,
                    None => continue,
                },
            };

            // API-order first genre, lowercased — stored as firstGenre and
            // compared against the next sync's first-in-API-order genre to
            // detect genre-only edits. MUST be API order, not alphabetical,
            // or every multi-genre album looks "changed" on every sync.
            let api_genre = item
                .genre
                .as_ref()
                .and_then(|g| g.first())
                .map(|g| g.tag.to_lowercase());

            let is_changed = if incremental {
                if let Some(info) = cached.get(&item.rating_key) {
                    let timestamp_changed = info.updated_at != item.updated_at;
                    // Only compare when we have a cached value. NULL
                    // firstGenre means the row predates this column — trust
                    // updatedAt for this sync; the row will get a real
                    // value written below.
                    let cached_genre = info.first_genre.as_ref().map(|g| g.to_lowercase());
                    let genre_changed = cached_genre.is_some() && api_genre != cached_genre;
                    timestamp_changed || genre_changed
                } else {
                    true // new album
                }
            } else {
                true
            };

            if is_changed {
                changed.push(AlbumUpsertRow {
                    title: item.title.clone(),
                    artist_id,
                    year: item.year,
                    source_id: item.rating_key.clone(),
                    art_url: item.thumb.clone(),
                    updated_at: item.updated_at,
                    added_at: item.added_at,
                    last_viewed_at: item.last_viewed_at,
                    first_genre: api_genre.clone(),
                });
                changed_ids.insert(item.rating_key.clone());

                // Collect shallow genre link (list views return only 1 genre)
                if let Some(genre) = item.genre.as_ref().and_then(|g| g.first()) {
                    genre_links.push((item.rating_key.clone(), genre.tag.clone()));
                }
            }
        }

        // Batch upsert changed albums
        let total = changed.len();
        for start in (0..total).step_by(BATCH_SIZE) {
            let end = (start + BATCH_SIZE).min(total);
            let chunk = &changed[start..end];
            let ids = self.cache.batch_upsert_albums(chunk)?;
            map.extend(ids);
            on_progress(SyncProgress {
                phase: SyncPhase::Albums,
                current: end,
                total,
                detail: format!("Albums: {}/{}", end, total),
            });
        }

        // Batch upsert genre links
        if !genre_links.is_empty() {
            let link_rows: Vec<(i64, Vec<String>)> = genre_links
                .into_iter()
                .filter_map(|(source_id, genre_name)| {
                    map.get(&source_id).map(|&id| (id, vec![genre_name]))
                })
                .collect();
            self.cache.batch_upsert_genres_and_links(&link_rows)?;
        }

        Ok((map, changed_ids))
    }

    // --- Phase 3: Tracks ---

    fn sync_tracks(
        &self,
        items: &[MediaItem],
        album_map: &HashMap<String, i64>,
        artist_map: &HashMap<String, i64>,
        cached: &HashMap<String, crate::cache::db::CachedItemInfo>,
        incremental: bool,
        on_progress: &dyn Fn(SyncProgress),
    ) -> Result<(), SyncError> {
        let mut changed: Vec<TrackUpsertRow> = Vec::new();

        for item in items {
            if incremental {
                if let Some(info) = cached.get(&item.rating_key) {
                    if info.updated_at == item.updated_at {
                        continue;
                    }
                }
            }

            let album_key = item.parent_rating_key.as_deref().unwrap_or("");
            let artist_key = item.grandparent_rating_key.as_deref().unwrap_or("");

            let album_id = match album_map.get(album_key) {
                Some(&id) => id,
                None => match self.cache.album_id(album_key)? {
                    Some(id) => id,
                    None => continue,
                },
            };
            let artist_id = match artist_map.get(artist_key) {
                Some(&id) => id,
                None => match self.cache.artist_id(artist_key)? {
                    Some(id) => id,
                    None => continue,
                },
            };

            // Audio stream extraction
            let audio_stream = item
                .media
                .as_ref()
                .and_then(|m| m.first())
                .and_then(|m| m.parts.as_ref())
                .and_then(|p| p.first())
                .and_then(|p| p.streams.as_ref())
                .and_then(|s| s.iter().find(|s| s.stream_type == Some(2)));

            let codec = audio_stream
                .and_then(|s| s.codec.clone())
                .or_else(|| {
                    item.media
                        .as_ref()
                        .and_then(|m| m.first())
                        .and_then(|m| m.audio_codec.clone())
                });
            let bitrate = audio_stream
                .and_then(|s| s.bitrate)
                .or_else(|| {
                    item.media
                        .as_ref()
                        .and_then(|m| m.first())
                        .and_then(|m| m.bitrate)
                });
            let part_key = item
                .media
                .as_ref()
                .and_then(|m| m.first())
                .and_then(|m| m.parts.as_ref())
                .and_then(|p| p.first())
                .and_then(|p| p.key.clone());
            let stream_id = audio_stream.and_then(|s| s.id);

            changed.push(TrackUpsertRow {
                title: item.title.clone(),
                album_id,
                artist_id,
                track_number: item.index,
                disc_number: item.parent_index,
                duration_ms: item.duration,
                source_id: item.rating_key.clone(),
                codec,
                part_key,
                stream_id,
                user_rating: item.user_rating,
                bitrate,
                track_artist: item.original_title.clone(),
                updated_at: item.updated_at,
            });
        }

        // Batch upsert
        let total = changed.len();
        for start in (0..total).step_by(BATCH_SIZE) {
            let end = (start + BATCH_SIZE).min(total);
            let chunk = &changed[start..end];
            self.cache.batch_upsert_tracks(chunk)?;
            on_progress(SyncProgress {
                phase: SyncPhase::Tracks,
                current: end,
                total,
                detail: format!("Tracks: {}/{}", end, total),
            });
        }

        Ok(())
    }

    // --- Phase 4: Deep Genre Sync ---

    async fn deep_genre_sync(
        &self,
        album_map: &HashMap<String, i64>,
        only_source_ids: Option<&HashSet<String>>,
        on_progress: Arc<dyn Fn(SyncProgress) + Send + Sync>,
    ) -> Result<(), SyncError> {
        let entries: Vec<(String, i64)> = match only_source_ids {
            Some(filter) => album_map
                .iter()
                .filter(|(k, _)| filter.contains(*k))
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
            None => album_map
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
        };

        let total = entries.len();
        if total == 0 {
            return Ok(());
        }

        let semaphore = Arc::new(Semaphore::new(DEEP_GENRE_CONCURRENCY));
        let completed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut handles = Vec::new();

        for (source_id, album_id) in entries {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let client = self.client.clone();
            let cache = self.cache.clone();
            let completed = completed.clone();
            let on_progress = on_progress.clone();

            handles.push(tokio::spawn(async move {
                let result =
                    process_album_deep_sync(&source_id, album_id, &client, &cache).await;
                drop(permit);

                match result {
                    Ok(()) => {}
                    Err(SyncError::Plex(PlexClientError::Unauthorized)) => {
                        return Err(SyncError::Plex(PlexClientError::Unauthorized));
                    }
                    Err(SyncError::Plex(PlexClientError::NotConnected)) => {
                        return Err(SyncError::Plex(PlexClientError::NotConnected));
                    }
                    Err(_) => {
                        // Skip individual album failures
                    }
                }

                let count = completed
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    + 1;
                if count.is_multiple_of(50) || count == total {
                    on_progress(SyncProgress {
                        phase: SyncPhase::DeepGenres,
                        current: count,
                        total,
                        detail: format!("Genre sync: {}/{}", count, total),
                    });
                }

                Ok(())
            }));
        }

        for handle in handles {
            match handle.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(e),
                Err(_join_err) => {} // task panicked, skip
            }
        }

        Ok(())
    }
}

/// Process a single album's deep metadata fetch + DB write.
async fn process_album_deep_sync(
    source_id: &str,
    album_id: i64,
    client: &PlexClient,
    cache: &CacheDatabase,
) -> Result<(), SyncError> {
    let metadata = client.fetch_item_metadata(source_id).await?;
    let genres = metadata.genre.unwrap_or_default();

    let genre_names: Vec<String> = genres.into_iter().map(|g| g.tag).collect();

    let colors_json = metadata
        .ultra_blur_colors
        .as_ref()
        .and_then(|c| serde_json::to_string(c).ok());

    cache.update_album_deep_metadata(
        album_id,
        &genre_names,
        metadata.user_rating,
        metadata.studio.as_deref(),
        colors_json.as_deref(),
    )?;

    Ok(())
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::db::CacheDatabase;
    use crate::plex::client::PlexClient;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use url::Url;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn setup_client(server_uri: &str) -> Arc<PlexClient> {
        let client = PlexClient::new("test-id".into());
        {
            let url = Url::parse(server_uri).unwrap();
            client.set_server_url(Some(url));
            client.set_token(Some("test-token".into()));
        }
        Arc::new(client)
    }

    fn media_item_json(
        rating_key: &str,
        title: &str,
        updated_at: i64,
        parent_rk: Option<&str>,
        grandparent_rk: Option<&str>,
    ) -> serde_json::Value {
        let mut obj = serde_json::json!({
            "ratingKey": rating_key,
            "title": title,
            "updatedAt": updated_at,
        });
        if let Some(prk) = parent_rk {
            obj["parentRatingKey"] = serde_json::json!(prk);
        }
        if let Some(grk) = grandparent_rk {
            obj["grandparentRatingKey"] = serde_json::json!(grk);
        }
        obj
    }

    fn album_item_json(
        rating_key: &str,
        title: &str,
        updated_at: i64,
        parent_rk: &str,
        genre: Option<&str>,
    ) -> serde_json::Value {
        let mut obj = media_item_json(rating_key, title, updated_at, Some(parent_rk), None);
        if let Some(g) = genre {
            obj["Genre"] = serde_json::json!([{"tag": g}]);
        }
        obj
    }

    fn track_item_json(
        rating_key: &str,
        title: &str,
        updated_at: i64,
        album_rk: &str,
        artist_rk: &str,
    ) -> serde_json::Value {
        let mut obj = media_item_json(rating_key, title, updated_at, Some(album_rk), Some(artist_rk));
        obj["duration"] = serde_json::json!(240000);
        obj["Media"] = serde_json::json!([{
            "audioCodec": "flac",
            "bitrate": 1411,
            "Part": [{"key": "/library/parts/1/file.flac"}]
        }]);
        obj
    }

    // Mount mocks for a simple library: 1 artist, 1 album, 1 track
    async fn mount_simple_library(server: &MockServer) {
        // Artists (type=8)
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "8"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [
                        media_item_json("ar1", "Radiohead", 1000, None, None)
                    ]
                }
            })))
            .mount(server)
            .await;

        // Albums (type=9)
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [
                        album_item_json("al1", "OK Computer", 1000, "ar1", Some("Rock"))
                    ]
                }
            })))
            .mount(server)
            .await;

        // Tracks (type=10)
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [
                        track_item_json("tr1", "Paranoid Android", 1000, "al1", "ar1")
                    ]
                }
            })))
            .mount(server)
            .await;

        // Deep metadata for album
        Mock::given(method("GET"))
            .and(path("/library/metadata/al1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "al1",
                        "title": "OK Computer",
                        "Genre": [{"tag": "Rock"}, {"tag": "Alternative Rock"}],
                        "userRating": 8.0,
                        "studio": "Parlophone"
                    }]
                }
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn test_full_sync() {
        let server = MockServer::start().await;
        mount_simple_library(&server).await;

        let cache = Arc::new(CacheDatabase::open_in_memory().unwrap());
        let client = setup_client(&server.uri());
        let engine = SyncEngine::new(cache.clone(), client);

        let progress_count = Arc::new(AtomicUsize::new(0));
        let pc = progress_count.clone();

        engine
            .full_sync("1", move |_p| {
                pc.fetch_add(1, Ordering::Relaxed);
            })
            .await
            .unwrap();

        // Verify data was synced
        let stats = cache.cache_stats().unwrap();
        assert_eq!(stats.artist_count, 1);
        assert_eq!(stats.album_count, 1);
        assert_eq!(stats.track_count, 1);
        assert!(stats.genre_count >= 1); // at least Rock from shallow + deep

        // Verify deep genres were fetched
        let genres = cache.album_genres("al1").unwrap();
        assert!(genres.contains(&"Rock".into()));
        assert!(genres.contains(&"Alternative Rock".into()));

        // Verify progress was called
        assert!(progress_count.load(Ordering::Relaxed) > 0);
    }

    #[tokio::test]
    async fn test_incremental_sync_skips_unchanged() {
        let server = MockServer::start().await;
        mount_simple_library(&server).await;

        let cache = Arc::new(CacheDatabase::open_in_memory().unwrap());
        let client = setup_client(&server.uri());
        let engine = SyncEngine::new(cache.clone(), client.clone());

        // First: full sync
        engine.full_sync("1", |_| {}).await.unwrap();

        // Now run incremental — nothing changed, so no deep fetches should happen
        // We verify by checking that genre data is still the same
        let engine2 = SyncEngine::new(cache.clone(), client);
        engine2.incremental_sync("1", |_| {}).await.unwrap();

        let stats = cache.cache_stats().unwrap();
        assert_eq!(stats.artist_count, 1);
        assert_eq!(stats.album_count, 1);
        assert_eq!(stats.track_count, 1);
    }

    #[tokio::test]
    async fn test_genre_change_detection() {
        let server = MockServer::start().await;
        mount_simple_library(&server).await;

        let cache = Arc::new(CacheDatabase::open_in_memory().unwrap());
        let client = setup_client(&server.uri());
        let engine = SyncEngine::new(cache.clone(), client);

        // Full sync first
        engine.full_sync("1", |_| {}).await.unwrap();

        // Now set up a new server where the album's genre changed but updatedAt didn't
        let server2 = MockServer::start().await;

        // Artists unchanged
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "8"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [
                        media_item_json("ar1", "Radiohead", 1000, None, None)
                    ]
                }
            })))
            .mount(&server2)
            .await;

        // Album: same updatedAt but different genre
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [
                        album_item_json("al1", "OK Computer", 1000, "ar1", Some("Electronic"))
                    ]
                }
            })))
            .mount(&server2)
            .await;

        // Tracks unchanged
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [
                        track_item_json("tr1", "Paranoid Android", 1000, "al1", "ar1")
                    ]
                }
            })))
            .mount(&server2)
            .await;

        // Deep metadata with new genre
        Mock::given(method("GET"))
            .and(path("/library/metadata/al1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "al1",
                        "title": "OK Computer",
                        "Genre": [{"tag": "Electronic"}, {"tag": "Experimental"}]
                    }]
                }
            })))
            .mount(&server2)
            .await;

        let client2 = setup_client(&server2.uri());
        let engine2 = SyncEngine::new(cache.clone(), client2);

        engine2.incremental_sync("1", |_| {}).await.unwrap();

        // Verify genres were updated
        let genres = cache.album_genres("al1").unwrap();
        assert!(genres.contains(&"Electronic".into()));
        assert!(genres.contains(&"Experimental".into()));
    }

    #[tokio::test]
    async fn test_progress_fraction() {
        let p = SyncProgress {
            phase: SyncPhase::Artists,
            current: 50,
            total: 100,
            detail: "test".into(),
        };
        assert!((p.fraction() - 0.5).abs() < f64::EPSILON);

        let p_zero = SyncProgress {
            phase: SyncPhase::Artists,
            current: 0,
            total: 0,
            detail: "test".into(),
        };
        assert!((p_zero.fraction()).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_deep_genre_concurrent_fetch() {
        let server = MockServer::start().await;

        // Set up 10 albums for concurrent deep fetch
        let mut album_metadata = Vec::new();
        for i in 0..10 {
            album_metadata.push(serde_json::json!({
                "ratingKey": format!("al{}", i),
                "title": format!("Album {}", i),
                "updatedAt": 1000,
                "parentRatingKey": "ar1",
                "Genre": [{"tag": "Rock"}]
            }));
        }

        // Artists
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "8"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [media_item_json("ar1", "Artist", 1000, None, None)]
                }
            })))
            .mount(&server)
            .await;

        // Albums
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": { "Metadata": album_metadata }
            })))
            .mount(&server)
            .await;

        // Tracks (empty)
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": { "Metadata": [] }
            })))
            .mount(&server)
            .await;

        // Deep metadata for each album
        for i in 0..10 {
            Mock::given(method("GET"))
                .and(path(format!("/library/metadata/al{}", i)))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "MediaContainer": {
                        "Metadata": [{
                            "ratingKey": format!("al{}", i),
                            "title": format!("Album {}", i),
                            "Genre": [{"tag": "Rock"}, {"tag": format!("Genre{}", i)}]
                        }]
                    }
                })))
                .mount(&server)
                .await;
        }

        let cache = Arc::new(CacheDatabase::open_in_memory().unwrap());
        let client = setup_client(&server.uri());
        let engine = SyncEngine::new(cache.clone(), client);

        engine.full_sync("1", |_| {}).await.unwrap();

        let stats = cache.cache_stats().unwrap();
        assert_eq!(stats.album_count, 10);
        // Each album has 2 genres: "Rock" + "GenreN"
        // Rock is shared, so total unique genres = 1 + 10 = 11
        assert_eq!(stats.genre_count, 11);
    }
}
