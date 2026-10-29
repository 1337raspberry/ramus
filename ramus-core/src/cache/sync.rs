use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::cache::db::{CacheDatabase, CacheError};
use crate::cache::sync_state::AlbumUserState;
use crate::cache::upsert::{AlbumUpsertRow, TrackUpsertRow};
use crate::plex::client::{MediaItem, PlexClient, PlexClientError};
use crate::util::dedup_case_insensitive;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncPhase {
    Artists,
    Albums,
    Tracks,
    DeepGenres,
    Done,
    /// Terminal failure frame. The engine itself returns `Err` rather than
    /// emitting this; callers (manual sync commands, auto-sync) emit it so
    /// progress listeners can settle instead of showing a stuck banner.
    Error,
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

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("plex error: {0}")]
    Plex(#[from] PlexClientError),
    #[error("cache error: {0}")]
    Cache(#[from] CacheError),
}

const BATCH_SIZE: usize = 500;
const DEEP_GENRE_CONCURRENCY: usize = 8;
/// How far before the previous sync's start a quick sync's "changed since"
/// filters reach. `>>=` is strictly greater, and re-fetching a few rows that
/// are already current costs nothing.
const CHANGED_SINCE_MARGIN_SECS: i64 = 300;
/// Past this many albums to recheck, one full track listing is cheaper than
/// fetching each album's tracks (~0.2 s per album, ~0.35 s per 200 tracks).
const MAX_ALBUM_RECHECKS: usize = 100;

pub struct SyncEngine {
    pub cache: Arc<CacheDatabase>,
    pub client: Arc<PlexClient>,
}

impl SyncEngine {
    pub fn new(cache: Arc<CacheDatabase>, client: Arc<PlexClient>) -> Self {
        Self { cache, client }
    }

    /// Full sync: all artists/albums/tracks + deep genre fetch for ALL albums.
    /// `include_styles` merges Plex `Style` tags into the genre table.
    pub async fn full_sync<F>(
        &self,
        library_key: &str,
        include_styles: bool,
        on_progress: F,
    ) -> Result<(), SyncError>
    where
        F: Fn(SyncProgress) + Send + Sync + 'static,
    {
        let on_progress = Arc::new(on_progress);
        let (album_map, _) = self
            .core_sync(library_key, false, on_progress.clone())
            .await?;

        self.deep_genre_sync(&album_map, None, include_styles, on_progress.clone())
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
        include_styles: bool,
        on_progress: F,
    ) -> Result<(), SyncError>
    where
        F: Fn(SyncProgress) + Send + Sync + 'static,
    {
        let on_progress = Arc::new(on_progress);
        let (album_map, changed_source_ids) = self
            .core_sync(library_key, true, on_progress.clone())
            .await?;

        if !changed_source_ids.is_empty() {
            self.deep_genre_sync(
                &album_map,
                Some(&changed_source_ids),
                include_styles,
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

    async fn core_sync(
        &self,
        library_key: &str,
        incremental: bool,
        on_progress: Arc<dyn Fn(SyncProgress) + Send + Sync>,
    ) -> Result<(HashMap<String, i64>, HashSet<String>), SyncError> {
        let (cached_artists, cached_albums, cached_tracks) = if incremental {
            (
                self.cache.all_artist_timestamps().unwrap_or_default(),
                self.cache.all_album_timestamps().unwrap_or_default(),
                self.cache.all_track_timestamps().unwrap_or_default(),
            )
        } else {
            (HashMap::new(), HashMap::new(), HashMap::new())
        };

        // Read before anything is fetched, so a change made while this sync
        // runs is still newer than the mark it leaves for the next one. If
        // the clock can't be read, the previous mark stays: older is safe.
        let started_at = self.client.server_time().await.ok();
        let changed_since = if incremental && !cached_tracks.is_empty() {
            self.cache.sync_mark(library_key).unwrap_or(None)
        } else {
            None
        };

        let p = on_progress.clone();
        let artist_items = self
            .client
            .fetch_all_items_with_progress(library_key, 8, 200, move |fetched, total| {
                p(SyncProgress {
                    phase: SyncPhase::Artists,
                    current: fetched,
                    total,
                    detail: if total > 0 {
                        format!("Fetching artists: {}/{}", fetched, total)
                    } else {
                        format!("Fetching artists: {}...", fetched)
                    },
                });
            })
            .await?;
        let artist_map =
            self.sync_artists(&artist_items, &cached_artists, incremental, on_progress.as_ref())?;

        let p = on_progress.clone();
        let on_album_page = move |fetched, total| {
            p(SyncProgress {
                phase: SyncPhase::Albums,
                current: fetched,
                total,
                detail: if total > 0 {
                    format!("Fetching albums: {}/{}", fetched, total)
                } else {
                    format!("Fetching albums: {}...", fetched)
                },
            });
        };
        // The track delta needs each album's track count to find deletions.
        let album_items = if changed_since.is_some() {
            self.client
                .fetch_section_albums_with_progress(library_key, 200, on_album_page)
                .await?
        } else {
            self.client
                .fetch_all_items_with_progress(library_key, 9, 200, on_album_page)
                .await?
        };
        let (album_map, changed_source_ids) = self.sync_albums(
            &album_items,
            &artist_map,
            &cached_albums,
            incremental,
            on_progress.as_ref(),
        )?;

        let delta_track_ids = match changed_since {
            Some(since) => {
                self.delta_sync_tracks(
                    library_key,
                    since - CHANGED_SINCE_MARGIN_SECS,
                    &album_items,
                    &album_map,
                    &artist_map,
                    &cached_albums,
                    &cached_tracks,
                    on_progress.clone(),
                )
                .await?
            }
            None => None,
        };

        let plex_track_ids: HashSet<String> = match delta_track_ids {
            Some(ids) => ids,
            None => {
                let p = on_progress.clone();
                let track_items = self
                    .client
                    .fetch_all_items_with_progress(library_key, 10, 200, move |fetched, total| {
                        p(SyncProgress {
                            phase: SyncPhase::Tracks,
                            current: fetched,
                            total,
                            detail: if total > 0 {
                                format!("Fetching tracks: {}/{}", fetched, total)
                            } else {
                                format!("Fetching tracks: {}...", fetched)
                            },
                        });
                    })
                    .await?;
                self.sync_tracks(
                    &track_items,
                    &album_map,
                    &artist_map,
                    &cached_tracks,
                    incremental,
                    on_progress.as_ref(),
                )?;
                track_items.iter().map(|i| i.rating_key.clone()).collect()
            }
        };

        // Prune local items no longer on Plex. Artists and albums are always
        // listed in full; the track set comes from either the full listing or
        // the delta's per-album reconciliation, so it is authoritative too.
        let plex_artist_ids: HashSet<String> =
            artist_items.iter().map(|i| i.rating_key.clone()).collect();
        let plex_album_ids: HashSet<String> =
            album_items.iter().map(|i| i.rating_key.clone()).collect();

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

        if let Some(t) = started_at {
            self.cache.set_sync_mark(library_key, t)?;
        }

        Ok((album_map, changed_source_ids))
    }

    /// Quick-sync track phase: fetches only the tracks changed, played or
    /// rated after `since`, then reconciles each album's track count against
    /// the server's `leafCount` to find deletions. Returns the live track
    /// set, or `None` when the caller must fall back to the full listing (an
    /// album without a count, too many albums to recheck, or totals that
    /// still disagree after reconciling).
    #[allow(clippy::too_many_arguments)]
    async fn delta_sync_tracks(
        &self,
        library_key: &str,
        since: i64,
        album_items: &[MediaItem],
        album_map: &HashMap<String, i64>,
        artist_map: &HashMap<String, i64>,
        cached_albums: &HashMap<String, crate::cache::db::CachedAlbumInfo>,
        cached_tracks: &HashMap<String, crate::cache::db::CachedItemInfo>,
        on_progress: Arc<dyn Fn(SyncProgress) + Send + Sync>,
    ) -> Result<Option<HashSet<String>>, SyncError> {
        let mut leaf_counts: HashMap<&str, usize> = HashMap::new();
        for album in album_items {
            match album.leaf_count {
                Some(n) => leaf_counts.insert(album.rating_key.as_str(), n.max(0) as usize),
                None => return Ok(None),
            };
        }

        // Plays and ratings are per-user and never move a track's
        // `updatedAt`, so each needs its own filter.
        let since_str = since.to_string();
        let mut changed: HashMap<String, MediaItem> = HashMap::new();
        for (field, label) in [
            ("updatedAt>>", "changed"),
            ("lastViewedAt>>", "played"),
            ("lastRatedAt>>", "rated"),
        ] {
            let p = on_progress.clone();
            let items = self
                .client
                .fetch_all_items_filtered_with_progress(
                    library_key,
                    10,
                    &[(field, since_str.as_str())],
                    200,
                    move |fetched, total| {
                        p(SyncProgress {
                            phase: SyncPhase::Tracks,
                            current: fetched,
                            total,
                            detail: format!("Fetching {} tracks: {}/{}", label, fetched, total),
                        });
                    },
                )
                .await?;
            for item in items {
                changed.insert(item.rating_key.clone(), item);
            }
        }
        let changed: Vec<MediaItem> = changed.into_values().collect();
        // Not `incremental`: played and rated rows carry an unchanged
        // `updatedAt` and would be skipped as already current.
        self.sync_tracks(&changed, album_map, artist_map, cached_tracks, false, on_progress.as_ref())?;

        let local = self.cache.track_ids_by_album()?;
        let mut recheck: HashSet<&str> = leaf_counts
            .iter()
            .filter(|(album, &count)| local.get(**album).map_or(0, Vec::len) != count)
            .map(|(album, _)| *album)
            .collect();
        // An album that gained one track and lost another keeps its count, so
        // any existing album with a new track is checked as well.
        for track in &changed {
            if cached_tracks.contains_key(&track.rating_key) {
                continue;
            }
            if let Some(album) = track.parent_rating_key.as_deref() {
                if cached_albums.contains_key(album) {
                    if let Some((&key, _)) = leaf_counts.get_key_value(album) {
                        recheck.insert(key);
                    }
                }
            }
        }
        if recheck.len() > MAX_ALBUM_RECHECKS {
            return Ok(None);
        }

        let total = recheck.len();
        let mut stale: HashSet<String> = HashSet::new();
        for (i, album) in recheck.iter().enumerate() {
            on_progress(SyncProgress {
                phase: SyncPhase::Tracks,
                current: i,
                total,
                detail: format!("Checking albums: {}/{}", i, total),
            });
            let children = self.client.fetch_children(album).await?;
            let live: HashSet<&str> = children.iter().map(|c| c.rating_key.as_str()).collect();
            if let Some(ids) = local.get(*album) {
                stale.extend(ids.iter().filter(|id| !live.contains(id.as_str())).cloned());
            }
            self.sync_tracks(&children, album_map, artist_map, cached_tracks, false, &|_| {})?;
        }

        // Tracks of albums gone from the server are left out here and go
        // with their album in the prune.
        let live_ids: HashSet<String> = self
            .cache
            .track_ids_by_album()?
            .into_iter()
            .filter(|(album, _)| leaf_counts.contains_key(album.as_str()))
            .flat_map(|(_, ids)| ids)
            .filter(|id| !stale.contains(id))
            .collect();
        let expected: usize = leaf_counts.values().sum();
        if live_ids.len() != expected {
            log::info!(
                "quick sync: {} local tracks against {} on the server; listing all tracks",
                live_ids.len(),
                expected
            );
            return Ok(None);
        }
        Ok(Some(live_ids))
    }

    fn sync_artists(
        &self,
        items: &[MediaItem],
        cached: &HashMap<String, crate::cache::db::CachedItemInfo>,
        incremental: bool,
        on_progress: &dyn Fn(SyncProgress),
    ) -> Result<HashMap<String, i64>, SyncError> {
        let mut map: HashMap<String, i64> = HashMap::new();

        if incremental {
            for (source_id, info) in cached {
                map.insert(source_id.clone(), info.id);
            }
        }

        type ArtistTuple = (String, Option<String>, String, Option<String>, Option<String>, Option<String>, Option<i64>);
        let mut changed: Vec<ArtistTuple> = Vec::new();

        for item in items {
            if incremental {
                if let Some(info) = cached.get(&item.rating_key) {
                    if info.updated_at == item.updated_at {
                        continue;
                    }
                }
            }
            let country = item
                .country
                .as_ref()
                .map(|tags| tags.iter().map(|t| t.tag.as_str()).collect::<Vec<_>>().join(", "))
                .filter(|s| !s.is_empty());
            changed.push((
                item.title.clone(),
                item.title_sort.clone(),
                item.rating_key.clone(),
                item.thumb.clone(),
                item.summary.clone(),
                country,
                item.updated_at,
            ));
        }

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
                detail: format!("Saving artists: {}/{}", end, total),
            });
        }

        Ok(map)
    }

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

        if incremental {
            for (source_id, info) in cached {
                map.insert(source_id.clone(), info.id);
            }
        }

        let mut changed: Vec<AlbumUpsertRow> = Vec::new();
        let mut genre_links: Vec<(String, String)> = Vec::new();
        let mut collection_links: Vec<(String, Vec<String>)> = Vec::new();

        for item in items {
            let artist_key = item.parent_rating_key.as_deref().unwrap_or("");
            let artist_id = match artist_map.get(artist_key) {
                Some(&id) => id,
                None => match self.cache.artist_id(artist_key)? {
                    Some(id) => id,
                    None => continue,
                },
            };

            // API-order first genre, lowercased. Stored as firstGenre and
            // compared against the next sync's API-order first genre to detect
            // genre-only edits. Must be API order, not alphabetical, or every
            // multi-genre album looks changed on every sync.
            let api_genre = item
                .genre
                .as_ref()
                .and_then(|g| g.first())
                .map(|g| g.tag.to_lowercase());

            let api_collection = item
                .collection
                .as_ref()
                .and_then(|c| c.first())
                .map(|c| c.tag.to_lowercase());

            let is_changed = if incremental {
                if let Some(info) = cached.get(&item.rating_key) {
                    let timestamp_changed = info.updated_at != item.updated_at;
                    let cached_genre = info.first_genre.as_ref().map(|g| g.to_lowercase());
                    // Compare Option<String> directly: a cached None vs an
                    // api Some("Rock") (a genre added since the previous
                    // sync) MUST be detected as changed; any is_some() guard
                    // on the cached side would silently skip those albums.
                    let genre_changed = api_genre != cached_genre;
                    let cached_col = info.first_collection.as_ref().map(|c| c.to_lowercase());
                    let col_changed = api_collection != cached_col;
                    // Plex thumb paths carry a timestamp suffix that bumps
                    // when the user replaces album art. `updatedAt` does not
                    // always move on art-only edits, so compare the stored
                    // path directly to catch those.
                    let thumb_changed = info.art_url != item.thumb;
                    timestamp_changed || genre_changed || col_changed || thumb_changed
                } else {
                    true
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
                    first_collection: api_collection.clone(),
                    view_count: item.view_count,
                });
                changed_ids.insert(item.rating_key.clone());

                // List views return only the first genre; deep fetch fills in the rest
                // (and merges Style tags in if enabled).
                if let Some(genre) = item.genre.as_ref().and_then(|g| g.first()) {
                    genre_links.push((item.rating_key.clone(), genre.tag.clone()));
                }

                if let Some(cols) = &item.collection {
                    let names: Vec<String> = cols.iter().map(|c| c.tag.clone()).collect();
                    if !names.is_empty() {
                        collection_links.push((item.rating_key.clone(), names));
                    }
                }
            }
        }

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
                detail: format!("Saving albums: {}/{}", end, total),
            });
        }

        // Favourites and plays don't move `updatedAt`, so the skip above never
        // sees them. Every album is listed on every sync, and the listing
        // carries these fields, so compare them all.
        let user_state = self.cache.album_user_state()?;
        let user_changes: Vec<(String, AlbumUserState)> = items
            .iter()
            .filter_map(|item| {
                let listed = AlbumUserState {
                    rating: item.user_rating,
                    view_count: item.view_count,
                    last_viewed_at: item.last_viewed_at,
                };
                match user_state.get(&item.rating_key) {
                    Some(cached) if cached.same_as(&listed) => None,
                    Some(_) => Some((item.rating_key.clone(), listed)),
                    None => None,
                }
            })
            .collect();
        if !user_changes.is_empty() {
            self.cache.set_album_user_state(&user_changes)?;
        }

        if !genre_links.is_empty() {
            let link_rows: Vec<(i64, Vec<String>)> = genre_links
                .into_iter()
                .filter_map(|(source_id, genre_name)| {
                    map.get(&source_id).map(|&id| (id, vec![genre_name]))
                })
                .collect();
            self.cache.batch_upsert_genres_and_links(&link_rows)?;
        }

        if !collection_links.is_empty() {
            let col_rows: Vec<(i64, Vec<String>)> = collection_links
                .into_iter()
                .filter_map(|(source_id, names)| {
                    map.get(&source_id).map(|&id| (id, names))
                })
                .collect();
            self.cache.batch_upsert_collections_and_links(&col_rows)?;
        }

        Ok((map, changed_ids))
    }

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
            let first_part = item
                .media
                .as_ref()
                .and_then(|m| m.first())
                .and_then(|m| m.parts.as_ref())
                .and_then(|p| p.first());
            let part_key = first_part.and_then(|p| p.key.clone());
            let file_size_bytes = first_part.and_then(|p| p.size);
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
                file_size_bytes,
                rating_count: item.rating_count,
                view_count: item.view_count,
                last_viewed_at: item.last_viewed_at,
            });
        }

        let total = changed.len();
        for start in (0..total).step_by(BATCH_SIZE) {
            let end = (start + BATCH_SIZE).min(total);
            let chunk = &changed[start..end];
            self.cache.batch_upsert_tracks(chunk)?;
            on_progress(SyncProgress {
                phase: SyncPhase::Tracks,
                current: end,
                total,
                detail: format!("Saving tracks: {}/{}", end, total),
            });
        }

        Ok(())
    }

    async fn deep_genre_sync(
        &self,
        album_map: &HashMap<String, i64>,
        only_source_ids: Option<&HashSet<String>>,
        include_styles: bool,
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

        on_progress(SyncProgress {
            phase: SyncPhase::DeepGenres,
            current: 0,
            total,
            detail: format!("Genre sync: 0/{}", total),
        });

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
                let result = process_album_deep_sync(
                    &source_id,
                    album_id,
                    include_styles,
                    &client,
                    &cache,
                )
                .await;
                drop(permit);

                match result {
                    Ok(()) => {}
                    Err(SyncError::Plex(PlexClientError::Unauthorized)) => {
                        return Err(SyncError::Plex(PlexClientError::Unauthorized));
                    }
                    Err(SyncError::Plex(PlexClientError::NotConnected)) => {
                        return Err(SyncError::Plex(PlexClientError::NotConnected));
                    }
                    Err(_) => {}
                }

                let count = completed
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    + 1;
                if count.is_multiple_of(10) || count == total {
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
                Err(_join_err) => {}
            }
        }

        Ok(())
    }
}

/// Fetch deep metadata for a single album and write it to the cache.
async fn process_album_deep_sync(
    source_id: &str,
    album_id: i64,
    include_styles: bool,
    client: &PlexClient,
    cache: &CacheDatabase,
) -> Result<(), SyncError> {
    let metadata = client.fetch_item_metadata(source_id).await?;
    let genres = metadata.genre.unwrap_or_default();
    let collections = metadata.collection.unwrap_or_default();
    let styles = metadata.style.unwrap_or_default();

    let mut genre_names: Vec<String> = genres.into_iter().map(|g| g.tag).collect();
    if include_styles {
        genre_names.extend(styles.into_iter().map(|s| s.tag));
        genre_names = dedup_case_insensitive(genre_names);
    }
    let collection_names: Vec<String> = collections.into_iter().map(|c| c.tag).collect();

    let colors_json = metadata
        .ultra_blur_colors
        .as_ref()
        .and_then(|c| serde_json::to_string(c).ok());

    let format = metadata
        .format
        .as_ref()
        .and_then(|f| f.first())
        .map(|f| f.tag.clone());

    cache.update_album_deep_metadata(
        album_id,
        &genre_names,
        &collection_names,
        metadata.user_rating,
        metadata.studio.as_deref(),
        colors_json.as_deref(),
        format.as_deref(),
    )?;

    Ok(())
}

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

    async fn mount_simple_library(server: &MockServer) {
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
            .full_sync("1", true, move |_p| {
                pc.fetch_add(1, Ordering::Relaxed);
            })
            .await
            .unwrap();

        let stats = cache.cache_stats().unwrap();
        assert_eq!(stats.artist_count, 1);
        assert_eq!(stats.album_count, 1);
        assert_eq!(stats.track_count, 1);
        assert!(stats.genre_count >= 1);

        let genres = cache.album_genres("al1").unwrap();
        assert!(genres.contains(&"Rock".into()));
        assert!(genres.contains(&"Alternative Rock".into()));

        assert!(progress_count.load(Ordering::Relaxed) > 0);
    }

    #[tokio::test]
    async fn test_incremental_sync_skips_unchanged() {
        let server = MockServer::start().await;
        mount_simple_library(&server).await;

        let cache = Arc::new(CacheDatabase::open_in_memory().unwrap());
        let client = setup_client(&server.uri());
        let engine = SyncEngine::new(cache.clone(), client.clone());

        engine.full_sync("1", true, |_| {}).await.unwrap();

        let engine2 = SyncEngine::new(cache.clone(), client);
        engine2.incremental_sync("1", true, |_| {}).await.unwrap();

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

        engine.full_sync("1", true, |_| {}).await.unwrap();

        // Server2: same updatedAt, different genre.
        let server2 = MockServer::start().await;

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

        engine2.incremental_sync("1", true, |_| {}).await.unwrap();

        let genres = cache.album_genres("al1").unwrap();
        assert!(genres.contains(&"Electronic".into()));
        assert!(genres.contains(&"Experimental".into()));
    }

    /// Plex bumps the timestamp suffix on the `thumb` path when album art is
    /// replaced, but does not always touch `updatedAt`. Incremental sync must
    /// pick up that change so the cached `artUrl` (and therefore the URL we
    /// fetch) reflects the new image.
    #[tokio::test]
    async fn test_art_change_detection() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "8"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [media_item_json("ar1", "Radiohead", 1000, None, None)]
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "al1",
                        "title": "OK Computer",
                        "updatedAt": 1000,
                        "parentRatingKey": "ar1",
                        "thumb": "/library/metadata/al1/thumb/1000",
                        "Genre": [{"tag": "Rock"}]
                    }]
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [track_item_json("tr1", "Paranoid Android", 1000, "al1", "ar1")]
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/library/metadata/al1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "al1",
                        "title": "OK Computer",
                        "Genre": [{"tag": "Rock"}]
                    }]
                }
            })))
            .mount(&server)
            .await;

        let cache = Arc::new(CacheDatabase::open_in_memory().unwrap());
        let client = setup_client(&server.uri());
        let engine = SyncEngine::new(cache.clone(), client);

        engine.full_sync("1", true, |_| {}).await.unwrap();

        let cached = cache.all_album_timestamps().unwrap();
        assert_eq!(
            cached.get("al1").unwrap().art_url.as_deref(),
            Some("/library/metadata/al1/thumb/1000")
        );

        // Server2: same updatedAt, same genre, BUT a bumped thumb timestamp
        // (the user replaced the album art on Plex).
        let server2 = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "8"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [media_item_json("ar1", "Radiohead", 1000, None, None)]
                }
            })))
            .mount(&server2)
            .await;

        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "al1",
                        "title": "OK Computer",
                        "updatedAt": 1000,
                        "parentRatingKey": "ar1",
                        "thumb": "/library/metadata/al1/thumb/2000",
                        "Genre": [{"tag": "Rock"}]
                    }]
                }
            })))
            .mount(&server2)
            .await;

        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [track_item_json("tr1", "Paranoid Android", 1000, "al1", "ar1")]
                }
            })))
            .mount(&server2)
            .await;

        Mock::given(method("GET"))
            .and(path("/library/metadata/al1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "al1",
                        "title": "OK Computer",
                        "Genre": [{"tag": "Rock"}]
                    }]
                }
            })))
            .mount(&server2)
            .await;

        let client2 = setup_client(&server2.uri());
        let engine2 = SyncEngine::new(cache.clone(), client2);
        engine2.incremental_sync("1", true, |_| {}).await.unwrap();

        let cached_after = cache.all_album_timestamps().unwrap();
        assert_eq!(
            cached_after.get("al1").unwrap().art_url.as_deref(),
            Some("/library/metadata/al1/thumb/2000"),
            "incremental sync must pick up art-only edits when updatedAt is unchanged"
        );
    }

    /// Mounts a library where the album has `Genre: [Rock, Pop]` and
    /// `Style: [Pop, Indie Rock]`. With Style merging on, we expect the
    /// album to end up tagged with the union (3 distinct genres after
    /// case-insensitive dedup); off, we expect just the two raw genres.
    async fn mount_library_with_styles(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "8"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [media_item_json("ar1", "Artist", 1000, None, None)]
                }
            })))
            .mount(server)
            .await;

        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "al1",
                        "title": "Album",
                        "updatedAt": 1000,
                        "parentRatingKey": "ar1",
                        "Genre": [{"tag": "Rock"}, {"tag": "Pop"}],
                        "Style": [{"tag": "pop"}, {"tag": "Indie Rock"}]
                    }]
                }
            })))
            .mount(server)
            .await;

        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": { "Metadata": [] }
            })))
            .mount(server)
            .await;

        Mock::given(method("GET"))
            .and(path("/library/metadata/al1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "al1",
                        "title": "Album",
                        "Genre": [{"tag": "Rock"}, {"tag": "Pop"}],
                        "Style": [{"tag": "pop"}, {"tag": "Indie Rock"}]
                    }]
                }
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn test_styles_merged_when_enabled() {
        let server = MockServer::start().await;
        mount_library_with_styles(&server).await;

        let cache = Arc::new(CacheDatabase::open_in_memory().unwrap());
        let client = setup_client(&server.uri());
        let engine = SyncEngine::new(cache.clone(), client);

        engine.full_sync("1", true, |_| {}).await.unwrap();

        let mut genres = cache.album_genres("al1").unwrap();
        genres.sort();
        // "Pop" appears in both Genre and Style — must dedupe case-insensitively.
        assert_eq!(genres.len(), 3, "got: {:?}", genres);
        assert!(genres.iter().any(|g| g.eq_ignore_ascii_case("rock")));
        assert!(genres.iter().any(|g| g.eq_ignore_ascii_case("indie rock")));
        // First-occurrence casing wins: Genre's "Pop" beats Style's "pop".
        assert!(
            genres.iter().any(|g| g == "Pop"),
            "Genre casing must win over Style casing: {:?}",
            genres,
        );
    }

    #[tokio::test]
    async fn test_styles_excluded_when_disabled() {
        let server = MockServer::start().await;
        mount_library_with_styles(&server).await;

        let cache = Arc::new(CacheDatabase::open_in_memory().unwrap());
        let client = setup_client(&server.uri());
        let engine = SyncEngine::new(cache.clone(), client);

        engine.full_sync("1", false, |_| {}).await.unwrap();

        let mut genres = cache.album_genres("al1").unwrap();
        genres.sort();
        assert_eq!(genres.len(), 2, "got: {:?}", genres);
        assert!(genres.iter().any(|g| g.eq_ignore_ascii_case("rock")));
        assert!(genres.iter().any(|g| g.eq_ignore_ascii_case("pop")));
        assert!(!genres.iter().any(|g| g.eq_ignore_ascii_case("indie rock")));
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

        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": { "Metadata": album_metadata }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": { "Metadata": [] }
            })))
            .mount(&server)
            .await;

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

        engine.full_sync("1", true, |_| {}).await.unwrap();

        let stats = cache.cache_stats().unwrap();
        assert_eq!(stats.album_count, 10);
        // 2 genres per album ("Rock" + "GenreN"); "Rock" is shared → 11 unique.
        assert_eq!(stats.genre_count, 11);
    }

    /// Matches requests carrying the given query key, e.g. `updatedAt>>`.
    struct HasParam(&'static str);

    impl wiremock::Match for HasParam {
        fn matches(&self, req: &wiremock::Request) -> bool {
            req.url.query_pairs().any(|(k, _)| k == self.0)
        }
    }

    /// Matches a listing with none of the quick sync's "changed since" filters.
    struct Unfiltered;

    impl wiremock::Match for Unfiltered {
        fn matches(&self, req: &wiremock::Request) -> bool {
            !req.url.query_pairs().any(|(k, _)| k.ends_with(">>"))
        }
    }

    fn container(items: Vec<serde_json::Value>) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MediaContainer": { "Metadata": items }
        }))
    }

    fn counted_album_json(rating_key: &str, leaf_count: i64) -> serde_json::Value {
        let mut obj = album_item_json(rating_key, rating_key, 1000, "ar1", Some("Rock"));
        obj["leafCount"] = serde_json::json!(leaf_count);
        obj
    }

    /// Mocks shared by both syncs: server clock, artists, and album metadata
    /// for the deep genre fetch.
    async fn mount_quick_sync_common(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/identity"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "MediaContainer": { "size": 0 }
            })))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "8"))
            .respond_with(container(vec![media_item_json("ar1", "Radiohead", 1000, None, None)]))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(wiremock::matchers::path_regex(r"^/library/metadata/[^/]+$"))
            .respond_with(container(vec![serde_json::json!({
                "ratingKey": "al", "title": "Album", "Genre": [{"tag": "Rock"}]
            })]))
            .mount(server)
            .await;
    }

    /// Full sync of al1 holding tr1 and tr2, leaving the cache and a sync mark.
    async fn full_sync_two_tracks(server: &MockServer) -> Arc<CacheDatabase> {
        mount_quick_sync_common(server).await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "9"))
            .respond_with(container(vec![album_item_json("al1", "al1", 1000, "ar1", Some("Rock"))]))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "10"))
            .respond_with(container(vec![
                track_item_json("tr1", "One", 1000, "al1", "ar1"),
                track_item_json("tr2", "Two", 1000, "al1", "ar1"),
            ]))
            .mount(server)
            .await;

        let cache = Arc::new(CacheDatabase::open_in_memory().unwrap());
        SyncEngine::new(cache.clone(), setup_client(&server.uri()))
            .full_sync("1", true, |_| {})
            .await
            .unwrap();
        assert!(cache.sync_mark("1").unwrap().is_some());
        server.reset().await;
        mount_quick_sync_common(server).await;
        cache
    }

    /// Answers the three "changed since" listings of the quick sync.
    async fn mount_track_deltas(
        server: &MockServer,
        changed: Vec<serde_json::Value>,
        played: Vec<serde_json::Value>,
        rated: Vec<serde_json::Value>,
    ) {
        for (key, items) in [("updatedAt>>", changed), ("lastViewedAt>>", played), ("lastRatedAt>>", rated)] {
            Mock::given(method("GET"))
                .and(path("/library/sections/1/all"))
                .and(query_param("type", "10"))
                .and(HasParam(key))
                .respond_with(container(items))
                .mount(server)
                .await;
        }
    }

    async fn mount_section_albums(server: &MockServer, albums: Vec<serde_json::Value>) {
        Mock::given(method("GET"))
            .and(path("/library/sections/1/albums"))
            .respond_with(container(albums))
            .mount(server)
            .await;
    }

    async fn mount_children(server: &MockServer, album: &str, tracks: Vec<serde_json::Value>) {
        Mock::given(method("GET"))
            .and(path(format!("/library/metadata/{album}/children")))
            .respond_with(container(tracks))
            .mount(server)
            .await;
    }

    async fn quick_sync(server: &MockServer, cache: &Arc<CacheDatabase>) {
        SyncEngine::new(cache.clone(), setup_client(&server.uri()))
            .incremental_sync("1", true, |_| {})
            .await
            .unwrap();
    }

    fn source_ids(cache: &CacheDatabase, album: &str) -> Vec<String> {
        let mut ids: Vec<String> = cache
            .tracks_for_album(album)
            .unwrap()
            .into_iter()
            .map(|t| t.rating_key)
            .collect();
        ids.sort();
        ids
    }

    async fn requests_to(server: &MockServer, path: &str) -> usize {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == path)
            .count()
    }

    // The unfiltered track listing is not mounted after the full sync, so
    // any of these quick syncs falling back to it fails with a 404.

    #[tokio::test]
    async fn quick_sync_fetches_only_changed_tracks() {
        let server = MockServer::start().await;
        let cache = full_sync_two_tracks(&server).await;
        mount_section_albums(&server, vec![counted_album_json("al1", 2), counted_album_json("al2", 1)]).await;
        mount_track_deltas(&server, vec![track_item_json("tr3", "Three", 2000, "al2", "ar1")], vec![], vec![]).await;

        quick_sync(&server, &cache).await;

        assert_eq!(source_ids(&cache, "al1"), ["tr1", "tr2"]);
        assert_eq!(source_ids(&cache, "al2"), ["tr3"]);
        // A brand-new album's count already matches; nothing to recheck.
        assert_eq!(requests_to(&server, "/library/metadata/al2/children").await, 0);
    }

    #[tokio::test]
    async fn quick_sync_applies_plays_and_ratings_from_other_devices() {
        let server = MockServer::start().await;
        let cache = full_sync_two_tracks(&server).await;
        mount_section_albums(&server, vec![counted_album_json("al1", 2)]).await;
        // Same `updatedAt` as the cached rows: plays and ratings don't move it.
        let mut played = track_item_json("tr1", "One", 1000, "al1", "ar1");
        played["viewCount"] = serde_json::json!(5);
        let mut rated = track_item_json("tr2", "Two", 1000, "al1", "ar1");
        rated["userRating"] = serde_json::json!(10.0);
        mount_track_deltas(&server, vec![], vec![played], vec![rated]).await;

        quick_sync(&server, &cache).await;

        assert_eq!(cache.track_by_source_id("tr1").unwrap().unwrap().view_count, Some(5));
        assert!(cache.track_by_source_id("tr2").unwrap().unwrap().is_favourite);
    }

    #[tokio::test]
    async fn quick_sync_prunes_a_deleted_track_from_the_album_count() {
        let server = MockServer::start().await;
        let cache = full_sync_two_tracks(&server).await;
        mount_section_albums(&server, vec![counted_album_json("al1", 1)]).await;
        mount_track_deltas(&server, vec![], vec![], vec![]).await;
        mount_children(&server, "al1", vec![track_item_json("tr1", "One", 1000, "al1", "ar1")]).await;

        quick_sync(&server, &cache).await;

        assert_eq!(source_ids(&cache, "al1"), ["tr1"]);
    }

    #[tokio::test]
    async fn quick_sync_rechecks_an_album_that_swapped_a_track() {
        let server = MockServer::start().await;
        let cache = full_sync_two_tracks(&server).await;
        // tr2 replaced by tr3: the count stays at 2.
        mount_section_albums(&server, vec![counted_album_json("al1", 2)]).await;
        let tr3 = track_item_json("tr3", "Three", 2000, "al1", "ar1");
        mount_track_deltas(&server, vec![tr3.clone()], vec![], vec![]).await;
        mount_children(&server, "al1", vec![track_item_json("tr1", "One", 1000, "al1", "ar1"), tr3]).await;

        quick_sync(&server, &cache).await;

        assert_eq!(source_ids(&cache, "al1"), ["tr1", "tr3"]);
    }

    #[tokio::test]
    async fn quick_sync_drops_tracks_of_a_removed_album() {
        let server = MockServer::start().await;
        let cache = full_sync_two_tracks(&server).await;
        mount_section_albums(&server, vec![]).await;
        mount_track_deltas(&server, vec![], vec![], vec![]).await;

        quick_sync(&server, &cache).await;

        assert_eq!(cache.cache_stats().unwrap().album_count, 0);
        assert_eq!(cache.cache_stats().unwrap().track_count, 0);
    }

    #[tokio::test]
    async fn quick_sync_lists_all_tracks_when_albums_carry_no_count() {
        let server = MockServer::start().await;
        let cache = full_sync_two_tracks(&server).await;
        mount_section_albums(&server, vec![album_item_json("al1", "al1", 1000, "ar1", Some("Rock"))]).await;
        mount_track_deltas(&server, vec![], vec![], vec![]).await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "10"))
            .and(Unfiltered)
            .respond_with(container(vec![track_item_json("tr1", "One", 1000, "al1", "ar1")]))
            .expect(1)
            .mount(&server)
            .await;

        quick_sync(&server, &cache).await;

        assert_eq!(source_ids(&cache, "al1"), ["tr1"]);
    }

    #[tokio::test]
    async fn quick_sync_lists_all_tracks_when_counts_disagree() {
        let server = MockServer::start().await;
        let cache = full_sync_two_tracks(&server).await;
        // Claims three tracks, but the album's children only return two.
        mount_section_albums(&server, vec![counted_album_json("al1", 3)]).await;
        mount_track_deltas(&server, vec![], vec![], vec![]).await;
        mount_children(
            &server,
            "al1",
            vec![
                track_item_json("tr1", "One", 1000, "al1", "ar1"),
                track_item_json("tr2", "Two", 1000, "al1", "ar1"),
            ],
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "10"))
            .and(Unfiltered)
            .respond_with(container(vec![
                track_item_json("tr1", "One", 1000, "al1", "ar1"),
                track_item_json("tr2", "Two", 1000, "al1", "ar1"),
            ]))
            .expect(1)
            .mount(&server)
            .await;

        quick_sync(&server, &cache).await;

        assert_eq!(source_ids(&cache, "al1"), ["tr1", "tr2"]);
    }

    #[tokio::test]
    async fn quick_sync_without_a_mark_lists_all_tracks() {
        // The existing incremental tests mount no `/identity`, so no mark is
        // ever written and every quick sync there takes the full listing.
        let server = MockServer::start().await;
        mount_simple_library(&server).await;
        let cache = Arc::new(CacheDatabase::open_in_memory().unwrap());
        let engine = SyncEngine::new(cache.clone(), setup_client(&server.uri()));
        engine.full_sync("1", true, |_| {}).await.unwrap();
        assert_eq!(cache.sync_mark("1").unwrap(), None);

        engine.incremental_sync("1", true, |_| {}).await.unwrap();

        assert_eq!(requests_to(&server, "/library/sections/1/albums").await, 0);
        assert_eq!(cache.cache_stats().unwrap().track_count, 1);
    }

    fn album_state(cache: &CacheDatabase, album: &str) -> AlbumUserState {
        cache.album_user_state().unwrap().remove(album).unwrap()
    }

    #[tokio::test]
    async fn sync_applies_album_favourites_and_plays_from_other_devices() {
        let server = MockServer::start().await;
        let cache = full_sync_two_tracks(&server).await;
        assert_eq!(album_state(&cache, "al1").rating, None);
        // Same `updatedAt` as the cached row: neither moves it.
        let mut al1 = counted_album_json("al1", 2);
        al1["userRating"] = serde_json::json!(10.0);
        al1["viewCount"] = serde_json::json!(3);
        al1["lastViewedAt"] = serde_json::json!(5000);
        mount_section_albums(&server, vec![al1]).await;
        mount_track_deltas(&server, vec![], vec![], vec![]).await;

        quick_sync(&server, &cache).await;

        let state = album_state(&cache, "al1");
        assert_eq!(state.rating, Some(10.0));
        assert_eq!(state.view_count, Some(3));
        assert_eq!(state.last_viewed_at, Some(5000));
        assert!(cache.album_by_source_id("al1").unwrap().unwrap().is_favourite);
    }

    #[tokio::test]
    async fn sync_clears_an_album_favourite_removed_elsewhere() {
        let server = MockServer::start().await;
        let cache = full_sync_two_tracks(&server).await;
        cache
            .set_album_user_state(&[(
                "al1".into(),
                AlbumUserState { rating: Some(10.0), view_count: Some(2), last_viewed_at: Some(4000) },
            )])
            .unwrap();
        // Unrated and marked unplayed on the server: the fields are absent.
        mount_section_albums(&server, vec![counted_album_json("al1", 2)]).await;
        mount_track_deltas(&server, vec![], vec![], vec![]).await;

        quick_sync(&server, &cache).await;

        assert_eq!(
            album_state(&cache, "al1"),
            AlbumUserState { rating: None, view_count: None, last_viewed_at: None }
        );
    }

    #[tokio::test]
    async fn full_sync_keeps_listed_album_favourites() {
        let server = MockServer::start().await;
        mount_quick_sync_common(&server).await;
        let mut al1 = album_item_json("al1", "al1", 1000, "ar1", Some("Rock"));
        al1["userRating"] = serde_json::json!(10.0);
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "9"))
            .respond_with(container(vec![al1]))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/library/sections/1/all"))
            .and(query_param("type", "10"))
            .respond_with(container(vec![track_item_json("tr1", "One", 1000, "al1", "ar1")]))
            .mount(&server)
            .await;
        let cache = Arc::new(CacheDatabase::open_in_memory().unwrap());

        SyncEngine::new(cache.clone(), setup_client(&server.uri()))
            .full_sync("1", true, |_| {})
            .await
            .unwrap();

        // The deep fetch's metadata mock carries no rating; the listing's stands.
        assert_eq!(album_state(&cache, "al1").rating, Some(10.0));
    }

    #[tokio::test]
    async fn quick_sync_filters_from_the_previous_start_minus_margin() {
        let server = MockServer::start().await;
        let cache = full_sync_two_tracks(&server).await;
        let mark = cache.sync_mark("1").unwrap().unwrap();
        mount_section_albums(&server, vec![counted_album_json("al1", 2)]).await;
        mount_track_deltas(&server, vec![], vec![], vec![]).await;

        quick_sync(&server, &cache).await;

        let expected = (mark - CHANGED_SINCE_MARGIN_SECS).to_string();
        let requests = server.received_requests().await.unwrap();
        for key in ["updatedAt>>", "lastViewedAt>>", "lastRatedAt>>"] {
            let sent = requests
                .iter()
                .find_map(|r| r.url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned()));
            assert_eq!(sent.as_deref(), Some(expected.as_str()), "{key}");
        }
    }
}
