//! User-initiated download commands.
//!
//! These commands build `UserDownloadJob` records and hand them to the
//! download worker (see `prefetch.rs`). Actual HTTP work happens in the
//! worker; the commands just translate rating keys into enqueued jobs.

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{AppHandle, State};

use ramus_core::cache::image_cache::ImageCache;
use ramus_core::models::{Album, Track};
use ramus_core::playback::transcode;
use ramus_core::playback::waveform;
use ramus_core::plex::client::PlexClient;
use ramus_core::search::parser::QueryParser;
use ramus_core::util::plex_art_url;

use crate::events::{emit_downloads_changed, ConnectionStatusPayload};
use crate::prefetch::{DownloadManagerSnapshot, UserDownloadJob};
use crate::state::AppState;

use super::{with_cache, CmdResult};

/// Sidecar path for a pre-computed waveform: `audio.flac.wave`. Same
/// pattern as `.spec` files — postcard-serialised `Vec<f32>` next to
/// the audio file.
pub fn waveform_sidecar_path(audio_path: &Path) -> PathBuf {
    let mut s = audio_path.as_os_str().to_os_string();
    s.push(".wave");
    PathBuf::from(s)
}

/// Fetch a track's waveform from Plex and write it next to the audio
/// file. Best-effort — silent failures are fine because the frontend's
/// seek bar degrades gracefully to "no waveform".
pub async fn warm_waveform_sidecar(client: &Arc<PlexClient>, rating_key: &str, audio_path: &Path) {
    let wave_path = waveform_sidecar_path(audio_path);
    if wave_path.is_file() {
        return;
    }
    let Ok(Some(stream)) = client.fetch_audio_stream(rating_key).await else {
        return;
    };
    let Some(stream_id) = stream.id else {
        return;
    };
    let Ok(levels) = client.fetch_levels(stream_id, None).await else {
        return;
    };
    if levels.is_empty() {
        return;
    }
    let normalized = waveform::normalize_db_levels(&levels);
    let Ok(bytes) = postcard::to_stdvec(&normalized) else {
        return;
    };
    let _ = tokio::fs::write(&wave_path, bytes).await;
}

/// Read a cached waveform sidecar, returning `None` on any kind of miss
/// or corruption. Callers fall back to a live Plex fetch.
pub async fn read_waveform_sidecar(audio_path: &Path) -> Option<Vec<f32>> {
    let bytes = tokio::fs::read(waveform_sidecar_path(audio_path))
        .await
        .ok()?;
    postcard::from_bytes::<Vec<f32>>(&bytes).ok()
}

/// Album-art sizes warmed at download time. Shared with
/// `recompute_image_pins` so the pin set covers every size that
/// `warm_art_cache` writes.
pub const WARM_ART_SIZES: &[u32] = &[72, 300, 1200];

/// Pre-fetch album art at all 3 display sizes into the on-disk image
/// cache. Used at user-download time so offline playback has art ready
/// for every surface that shows it (now playing 1200, grid 300, queue 72).
/// Best-effort — a single size failing doesn't abort the others.
///
/// Inserted entries are pinned so the LRU won't evict them when online
/// browsing pushes other art into the cache. Pins are released by
/// `recompute_image_pins` after the underlying download is removed.
pub async fn warm_art_cache(
    image_cache: &Arc<Mutex<ImageCache>>,
    client: &Arc<PlexClient>,
    http: &reqwest::Client,
    thumb: &str,
) {
    for &size in WARM_ART_SIZES {
        let needs_fetch = {
            let mut cache = image_cache.lock();
            match cache.get(thumb, size) {
                Some(_) => {
                    // Already on disk — just promote to pinned, no
                    // re-download needed.
                    cache.pin(thumb, size);
                    false
                }
                None => true,
            }
        };
        if !needs_fetch {
            continue;
        }
        let Some(server_url) = client.server_url() else {
            return;
        };
        let Some(token) = client.token() else {
            return;
        };
        let url = plex_art_url(&server_url, thumb, size);
        let Ok(response) = http.get(&url).header("X-Plex-Token", &token).send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(bytes) = response.bytes().await else {
            continue;
        };
        let mut cache = image_cache.lock();
        let _ = cache.insert_pinned(thumb, size, &bytes);
    }
}

/// Reset the image cache's pin set to match the union of art URLs
/// referenced by the current `downloads` table. Call after any download
/// removal so orphaned art becomes evictable, and at startup so
/// upgrades from older builds (whose cache meta has no pin flags) are
/// repaired.
pub fn recompute_image_pins(state: &AppState) {
    let thumbs = match state.cache.lock().as_ref() {
        Some(db) => match db.pinned_download_thumbs() {
            Ok(t) => t,
            Err(e) => {
                log::warn!("downloads: pinned_download_thumbs failed: {e}");
                return;
            }
        },
        None => return,
    };
    state
        .image_cache
        .lock()
        .set_pinned_thumbs(&thumbs, WARM_ART_SIZES);
}

/// Summary of the downloaded album for the Downloads panel. "Partial"
/// albums (some tracks downloaded, some not) still appear here with
/// `downloaded < total`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadedAlbumSummary {
    pub rating_key: String,
    pub title: String,
    pub artist_name: String,
    pub thumb: Option<String>,
    pub downloaded: u32,
    pub total: u32,
    pub size_bytes: i64,
}

/// Orphan track: a downloaded track whose parent album has only that one
/// track downloaded. Shown in the Downloads panel's "Individual Tracks"
/// section so partial-album rows don't duplicate into the tracks list.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadedTrackSummary {
    pub rating_key: String,
    pub album_rating_key: String,
    pub title: String,
    pub artist_name: String,
    pub album_title: String,
    pub thumb: Option<String>,
    pub size_bytes: i64,
    pub codec: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsOverview {
    pub in_progress: Option<crate::prefetch::InProgressDownload>,
    /// Capped preview of queued rating keys (for UI lists). See `queue_len`
    /// for the authoritative count.
    pub queue: Vec<String>,
    /// Total items in the user download queue — `queue.len()` is capped at
    /// a preview size by the worker's snapshot, so use this for counts.
    pub queue_len: usize,
    pub total_bytes: i64,
    pub albums: Vec<DownloadedAlbumSummary>,
    pub orphan_tracks: Vec<DownloadedTrackSummary>,
    /// Every downloaded track's rating key. Album-level summaries only
    /// expose counts, not individual track IDs — the frontend needs the
    /// full flat list to know whether a specific track is playable
    /// offline (e.g. to fade non-downloaded rows in album detail view).
    pub downloaded_rating_keys: Vec<String>,
}

// --- Start downloads ---

/// Enqueue a single track for download. Silently skipped if already
/// downloaded or if the track needs transcoding (user setting).
#[tauri::command]
pub async fn download_track(state: State<'_, AppState>, rating_key: String) -> CmdResult<()> {
    let track = lookup_track(&state, &rating_key)?;
    let job = build_job(&state, &track)?;
    state.prefetch_handle.queue_user_downloads(vec![job]);
    Ok(())
}

/// Enqueue every direct-play track on an album.
#[tauri::command]
pub async fn download_album(
    state: State<'_, AppState>,
    album_rating_key: String,
) -> CmdResult<usize> {
    let tracks = lookup_album_tracks(&state, &album_rating_key)?;
    let jobs: Vec<UserDownloadJob> = tracks
        .iter()
        .filter_map(|t| build_job(&state, t).ok())
        .collect();
    let n = jobs.len();
    state.prefetch_handle.queue_user_downloads(jobs);
    Ok(n)
}

/// Enqueue every favourited track.
#[tauri::command]
pub async fn download_all_starred_tracks(state: State<'_, AppState>) -> CmdResult<usize> {
    let tracks = with_cache(&state, |cache| cache.favourite_tracks())?;
    let jobs: Vec<UserDownloadJob> = tracks
        .iter()
        .filter_map(|t| build_job(&state, t).ok())
        .collect();
    let n = jobs.len();
    state.prefetch_handle.queue_user_downloads(jobs);
    Ok(n)
}

/// Enqueue every track on every favourited album.
#[tauri::command]
pub async fn download_all_starred_albums(state: State<'_, AppState>) -> CmdResult<usize> {
    let albums = with_cache(&state, |cache| cache.favourite_albums())?;
    let mut jobs: Vec<UserDownloadJob> = Vec::new();
    for album in &albums {
        let tracks = lookup_album_tracks(&state, &album.rating_key)?;
        for t in &tracks {
            if let Ok(job) = build_job(&state, t) {
                jobs.push(job);
            }
        }
    }
    let n = jobs.len();
    state.prefetch_handle.queue_user_downloads(jobs);
    Ok(n)
}

/// Enqueue every direct-play track for albums matching a saved-search query.
/// Mirrors the starred-albums pipeline: resolve query → album IDs → tracks,
/// then one job per track. Returns the number of jobs enqueued.
#[tauri::command]
pub async fn download_search_results(
    state: State<'_, AppState>,
    query: String,
) -> CmdResult<usize> {
    let albums = resolve_search_albums(&state, &query)?;
    let mut jobs: Vec<UserDownloadJob> = Vec::new();
    for album in &albums {
        let tracks = lookup_album_tracks(&state, &album.rating_key)?;
        for t in &tracks {
            if let Ok(job) = build_job(&state, t) {
                jobs.push(job);
            }
        }
    }
    let n = jobs.len();
    state.prefetch_handle.queue_user_downloads(jobs);
    Ok(n)
}

// --- Cancel ---

#[tauri::command]
pub async fn cancel_download(state: State<'_, AppState>, rating_key: String) -> CmdResult<()> {
    state.prefetch_handle.cancel_user_download(&rating_key);
    Ok(())
}

#[tauri::command]
pub async fn cancel_all_downloads(state: State<'_, AppState>) -> CmdResult<()> {
    state.prefetch_handle.cancel_all_user_downloads();
    Ok(())
}

// --- Remove ---

#[tauri::command]
pub async fn remove_download(
    app: AppHandle,
    state: State<'_, AppState>,
    rating_key: String,
) -> CmdResult<()> {
    let path = with_cache(&state, |cache| cache.remove_download(&rating_key))?;
    state.player.unregister_persistent_download(&rating_key);
    if let Some(p) = path {
        let _ = tokio::fs::remove_file(PathBuf::from(&p)).await;
        let spec = ramus_core::playback::spectrum::spec_file_path(std::path::Path::new(&p));
        let _ = tokio::fs::remove_file(spec).await;
    }
    recompute_image_pins(&state);
    emit_downloads_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn remove_album_downloads(
    app: AppHandle,
    state: State<'_, AppState>,
    album_rating_key: String,
) -> CmdResult<usize> {
    let paths = with_cache(&state, |cache| {
        cache.remove_album_downloads(&album_rating_key)
    })?;
    // Unregister each — we don't have the rating keys directly; re-query the
    // player map and drop anything whose file path was just deleted.
    let persistent = state.player.persistent_download_paths();
    let path_set: std::collections::HashSet<String> = paths.iter().cloned().collect();
    for (rk, p) in persistent {
        if path_set.contains(&p.to_string_lossy().to_string()) {
            state.player.unregister_persistent_download(&rk);
        }
    }
    for p in &paths {
        let _ = tokio::fs::remove_file(PathBuf::from(p)).await;
        let spec = ramus_core::playback::spectrum::spec_file_path(std::path::Path::new(p));
        let _ = tokio::fs::remove_file(spec).await;
    }
    recompute_image_pins(&state);
    emit_downloads_changed(&app);
    Ok(paths.len())
}

#[tauri::command]
pub async fn remove_all_downloads(app: AppHandle, state: State<'_, AppState>) -> CmdResult<usize> {
    let paths = with_cache(&state, |cache| cache.clear_all_downloads())?;
    state
        .player
        .rehydrate_persistent_cache(std::collections::HashMap::new());
    for p in &paths {
        let _ = tokio::fs::remove_file(PathBuf::from(p)).await;
        let spec = ramus_core::playback::spectrum::spec_file_path(std::path::Path::new(p));
        let _ = tokio::fs::remove_file(spec).await;
    }
    recompute_image_pins(&state);
    emit_downloads_changed(&app);
    Ok(paths.len())
}

// --- Read state ---

#[tauri::command]
pub async fn get_downloads_overview(state: State<'_, AppState>) -> CmdResult<DownloadsOverview> {
    let snapshot: DownloadManagerSnapshot = state.prefetch_handle.snapshot();

    // One locked scope for every DB read. The `with_cache` helper keeps the
    // guard alive for the whole closure, so the view is also internally
    // consistent (no chance of a row inserted / removed between queries).
    let (total_bytes, mut albums, orphan_tracks, downloaded_rating_keys) =
        with_cache(&state, |cache| {
            let total_bytes = cache.total_download_bytes()?;
            let album_counts = cache.downloaded_counts_by_album()?;
            let orphan_rows = cache.orphan_downloaded_tracks()?;
            let album_keys: Vec<String> = album_counts.keys().cloned().collect();
            let totals = cache.album_total_track_counts(&album_keys)?;
            let orphan_album_keys: std::collections::HashSet<String> = orphan_rows
                .iter()
                .map(|r| r.album_rating_key.clone())
                .collect();

            // N+1 album lookups — acceptable because the typical downloaded
            // list is small (<1000).
            let mut albums: Vec<DownloadedAlbumSummary> = Vec::new();
            for (k, (downloaded, size_bytes)) in &album_counts {
                if orphan_album_keys.contains(k.as_str()) {
                    continue;
                }
                let album = cache.album_by_source_id(k)?;
                let (title, artist_name, thumb) = match album {
                    Some(a) => (a.title, a.artist_name, a.thumb),
                    None => (k.clone(), String::new(), None),
                };
                albums.push(DownloadedAlbumSummary {
                    rating_key: k.clone(),
                    title,
                    artist_name,
                    thumb,
                    downloaded: *downloaded,
                    total: totals.get(k).copied().unwrap_or(*downloaded),
                    size_bytes: *size_bytes,
                });
            }

            let mut orphan_tracks: Vec<DownloadedTrackSummary> = Vec::new();
            for r in orphan_rows {
                let t = cache.track_by_source_id(&r.rating_key)?;
                let (title, artist_name, album_title, thumb) = match t {
                    Some(t) => (t.title, t.artist_name, t.album_title, t.thumb),
                    None => (r.rating_key.clone(), String::new(), String::new(), None),
                };
                orphan_tracks.push(DownloadedTrackSummary {
                    rating_key: r.rating_key,
                    album_rating_key: r.album_rating_key,
                    title,
                    artist_name,
                    album_title,
                    thumb,
                    size_bytes: r.size_bytes,
                    codec: r.codec,
                });
            }

            let downloaded_rating_keys: Vec<String> =
                cache.downloaded_rating_keys()?.into_iter().collect();

            Ok((total_bytes, albums, orphan_tracks, downloaded_rating_keys))
        })?;

    albums.sort_by(|a, b| {
        a.artist_name
            .to_lowercase()
            .cmp(&b.artist_name.to_lowercase())
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });

    Ok(DownloadsOverview {
        in_progress: snapshot.in_progress,
        queue: snapshot.queued,
        queue_len: snapshot.queue_len,
        total_bytes,
        albums,
        orphan_tracks,
        downloaded_rating_keys,
    })
}

/// Estimated bytes for downloading every favourited track. Uses actual
/// `fileSizeBytes` when known, otherwise `bitrate_kbps × duration_sec / 8`.
#[tauri::command]
pub async fn estimate_starred_tracks_size(state: State<'_, AppState>) -> CmdResult<i64> {
    let tracks = with_cache(&state, |cache| cache.favourite_tracks())?;
    Ok(estimate_total_bytes(&tracks))
}

#[tauri::command]
pub async fn estimate_starred_albums_size(state: State<'_, AppState>) -> CmdResult<i64> {
    let albums = with_cache(&state, |cache| cache.favourite_albums())?;
    let mut total: i64 = 0;
    for album in &albums {
        let tracks = lookup_album_tracks(&state, &album.rating_key)?;
        total += estimate_total_bytes(&tracks);
    }
    Ok(total)
}

/// Estimated bytes + track count for a saved-search query without hitting
/// the network. Same estimation rules as the starred helpers: real
/// `fileSizeBytes` where known, otherwise bitrate × duration.
#[tauri::command]
pub async fn estimate_search_size(
    state: State<'_, AppState>,
    query: String,
) -> CmdResult<SearchDownloadEstimate> {
    let albums = resolve_search_albums(&state, &query)?;
    let mut total_bytes: i64 = 0;
    let mut track_count: usize = 0;
    for album in &albums {
        let tracks = lookup_album_tracks(&state, &album.rating_key)?;
        track_count += tracks.len();
        total_bytes += estimate_total_bytes(&tracks);
    }
    Ok(SearchDownloadEstimate {
        total_bytes,
        track_count,
        album_count: albums.len(),
    })
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchDownloadEstimate {
    pub total_bytes: i64,
    pub track_count: usize,
    pub album_count: usize,
}

// --- Connection status ---

/// Current reachability snapshot for the frontend. Called once on mount to
/// sync initial state; subsequent changes arrive via the `connection-status`
/// event stream.
#[tauri::command]
pub async fn get_connection_status(
    state: State<'_, AppState>,
) -> CmdResult<ConnectionStatusPayload> {
    let effective_offline = state.effective_offline();
    let online = state
        .server_reachable
        .load(std::sync::atomic::Ordering::Acquire);
    let offline_mode_manual = state.settings.read().offline_mode;
    Ok(ConnectionStatusPayload {
        online,
        offline_mode_manual,
        effective_offline,
    })
}

// --- Helpers ---

fn lookup_track(state: &State<'_, AppState>, rating_key: &str) -> Result<Track, String> {
    with_cache(state, |cache| cache.track_by_source_id(rating_key))?
        .ok_or_else(|| format!("Track {rating_key} not found"))
}

fn lookup_album_tracks(
    state: &State<'_, AppState>,
    album_rating_key: &str,
) -> Result<Vec<Track>, String> {
    with_cache(state, |cache| cache.tracks_for_album(album_rating_key))
}

/// Build a direct-play download URL for a track. Returns `Err` for any
/// track that can't be downloaded (no part_key, codec-only-transcodable,
/// no server URL). User downloads always use direct play regardless of
/// the user's PlaybackMode setting — the whole point is local-first
/// playback offline.
fn build_job(state: &State<'_, AppState>, track: &Track) -> Result<UserDownloadJob, String> {
    let part_key = track
        .part_key
        .as_ref()
        .ok_or_else(|| "track has no part_key".to_string())?;
    let codec = track
        .codec
        .clone()
        .ok_or_else(|| "track has no codec".to_string())?;
    let server_url = state
        .client
        .server_url()
        .ok_or_else(|| "no server url".to_string())?;
    let token = state.client.token().ok_or_else(|| "no token".to_string())?;
    let url = transcode::build_direct_play_url(&server_url, part_key, &token)
        .ok_or_else(|| "could not build direct-play url".to_string())?;

    // Album key falls back to the track's rating key so an orphan track
    // still gets a stable grouping. `tracks_for_album` always populates
    // album_key.
    let album_rating_key = track
        .album_key
        .clone()
        .unwrap_or_else(|| track.rating_key.clone());

    Ok(UserDownloadJob {
        rating_key: track.rating_key.clone(),
        album_rating_key,
        title: track.title.clone(),
        artist_name: track.display_artist().to_string(),
        album_title: track.album_title.clone(),
        thumb: track.thumb.clone(),
        codec,
        url: url.to_string(),
        expected_size_bytes: track.file_size_bytes.map(|b| b as u64),
    })
}

/// Run a saved-search-style query through the search engine and return the
/// matching albums. Mirrors `commands::search::search_albums_for_grid` but
/// used from downloads where we need both album metadata and the full
/// track list per album.
fn resolve_search_albums(state: &State<'_, AppState>, query: &str) -> Result<Vec<Album>, String> {
    let parsed = QueryParser::parse(query);
    let ids = {
        let engine = state.search_engine.read();
        let engine = engine.as_ref().ok_or("Search engine not initialized")?;
        engine
            .search_album_ids(&parsed)
            .map_err(|e| e.to_string())?
    };
    with_cache(state, |cache| cache.albums_by_internal_ids(&ids))
}

fn estimate_total_bytes(tracks: &[Track]) -> i64 {
    tracks
        .iter()
        .map(|t| {
            if let Some(bytes) = t.file_size_bytes {
                bytes
            } else {
                match t.bitrate {
                    // bitrate is in kbps, duration is seconds.
                    Some(kbps) if kbps > 0 => ((kbps as f64 * 1000.0 * t.duration) / 8.0) as i64,
                    _ => 0,
                }
            }
        })
        .sum()
}
