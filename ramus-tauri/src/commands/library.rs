use tauri::State;

use ramus_core::cache::db::CacheStats;
use ramus_core::genre::node::GenreNode;
use ramus_core::models::{Album, AlbumColorInfo, AlbumFilterParams, ArtistInfo, Track, VibrantPalette};
use ramus_core::search::engine::GenreExpander;
use ramus_core::util::plex_art_url;
use rand::seq::SliceRandom;
use serde::Serialize;

use crate::state::AppState;

use super::{with_cache, CmdResult};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreTreeResponse {
    pub tree: Vec<GenreNode>,
    pub total_album_count: usize,
}

/// When effective-offline mode is active, returns the set of album rating
/// keys that have at least one downloaded track. `None` means "no filter —
/// return everything". Callers use this to restrict library views to what
/// the user can actually play without a server.
fn offline_album_source_ids(
    state: &State<'_, AppState>,
) -> CmdResult<Option<std::collections::HashSet<String>>> {
    if !state.effective_offline() {
        return Ok(None);
    }
    let ids = with_cache(state, |db| db.downloaded_album_source_ids())?;
    Ok(Some(ids))
}

fn filter_albums(
    albums: Vec<Album>,
    allowed: Option<std::collections::HashSet<String>>,
) -> Vec<Album> {
    match allowed {
        None => albums,
        Some(set) => albums
            .into_iter()
            .filter(|a| set.contains(&a.rating_key))
            .collect(),
    }
}

#[tauri::command]
pub async fn get_genre_tree(state: State<'_, AppState>) -> CmdResult<GenreTreeResponse> {
    let mut genre_album_sets = with_cache(&state, |db| db.genre_album_sets())?;

    // Offline mode: intersect every per-genre album set with the set of
    // albums that have at least one downloaded track, and drop any
    // genres that end up empty.
    if state.effective_offline() {
        let downloaded = with_cache(&state, |db| db.downloaded_album_internal_ids())?;
        genre_album_sets = genre_album_sets
            .into_iter()
            .filter_map(|(name, ids)| {
                let kept: std::collections::HashSet<i64> =
                    ids.intersection(&downloaded).copied().collect();
                if kept.is_empty() {
                    None
                } else {
                    Some((name, kept))
                }
            })
            .collect();
    }

    // Deduplicated total: union of all album IDs across genres.
    let total_album_count = {
        let mut all: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for ids in genre_album_sets.values() {
            all.extend(ids);
        }
        all.len()
    };

    let mapper = state.genre_mapper.read();
    let tree = if let Some(mapper) = mapper.as_ref() {
        mapper.build_display_tree(&genre_album_sets)
    } else {
        // Fallback: flat genre list when the mapper isn't loaded.
        let mut nodes: Vec<GenreNode> = genre_album_sets
            .iter()
            .map(|(name, ids)| GenreNode {
                id: name.clone(),
                name: name.clone(),
                short_summary: None,
                children: None,
                album_count: ids.len(),
                deduplicated_total_count: ids.len(),
            })
            .collect();
        nodes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        nodes
    };

    Ok(GenreTreeResponse {
        tree,
        total_album_count,
    })
}

#[tauri::command]
pub async fn get_albums_for_genre(
    state: State<'_, AppState>,
    genre: String,
) -> CmdResult<Vec<Album>> {
    // Resolve the canonical to its full subtree, then translate that subtree
    // back to user-library tags via AKA/fuzzy match — DB stores raw user tags
    // (e.g. "Hip-Hop"), so querying with canonical names alone misses anything
    // tagged via an AKA. Guard against bad fuzzy: if expanded doesn't include
    // the original genre, the mapper landed in the wrong family — fall back
    // to the raw name (covers "Other" bucket clicks).
    let mapper_guard = state.genre_mapper.read();
    let names: Vec<String> = match mapper_guard.as_ref() {
        Some(mapper) => match mapper.expand_genre(&genre) {
            Some(expanded) if expanded.iter().any(|n| n.eq_ignore_ascii_case(&genre)) => {
                let lower_subtree: std::collections::HashSet<String> =
                    expanded.iter().map(|s| s.to_lowercase()).collect();
                let user_tags = with_cache(&state, |db| db.genre_album_sets())?
                    .into_keys()
                    .collect::<Vec<_>>();
                let mut matching: Vec<String> = user_tags
                    .into_iter()
                    .filter(|tag| {
                        mapper
                            .match_all(tag)
                            .iter()
                            .any(|n| lower_subtree.contains(&n.name.to_lowercase()))
                    })
                    .collect();
                // Also include the canonical names themselves so verbatim
                // user-tagged albums still hit even if the inverse pass missed.
                matching.extend(expanded);
                matching
            }
            _ => vec![genre.clone()],
        },
        None => vec![genre.clone()],
    };
    drop(mapper_guard);

    let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let albums = with_cache(&state, |db| db.albums_for_genres(&name_refs))?;
    Ok(filter_albums(albums, offline_album_source_ids(&state)?))
}

/// Fetch albums for an explicit list of genre names (no expansion). Used by
/// the frontend when clicking parent nodes like "Other" where child names
/// are already known.
#[tauri::command]
pub async fn get_albums_for_genre_names(
    state: State<'_, AppState>,
    genres: Vec<String>,
) -> CmdResult<Vec<Album>> {
    let name_refs: Vec<&str> = genres.iter().map(|s| s.as_str()).collect();
    let albums = with_cache(&state, |db| db.albums_for_genres(&name_refs))?;
    Ok(filter_albums(albums, offline_album_source_ids(&state)?))
}

#[tauri::command]
pub async fn get_all_albums(state: State<'_, AppState>) -> CmdResult<Vec<Album>> {
    let albums = with_cache(&state, |db| db.all_albums())?;
    Ok(filter_albums(albums, offline_album_source_ids(&state)?))
}

#[tauri::command]
pub async fn get_favourite_tracks(state: State<'_, AppState>) -> CmdResult<Vec<Track>> {
    let tracks = with_cache(&state, |db| db.favourite_tracks())?;
    if state.effective_offline() {
        let downloaded = with_cache(&state, |db| db.downloaded_rating_keys())?;
        Ok(tracks
            .into_iter()
            .filter(|t| downloaded.contains(&t.rating_key))
            .collect())
    } else {
        Ok(tracks)
    }
}

#[tauri::command]
pub async fn get_albums_for_artist(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<Vec<Album>> {
    let albums = with_cache(&state, |db| db.albums_for_artist(&source_id))?;
    Ok(filter_albums(albums, offline_album_source_ids(&state)?))
}

#[tauri::command]
pub async fn get_albums_for_artist_name(
    state: State<'_, AppState>,
    name: String,
) -> CmdResult<Vec<Album>> {
    let albums = with_cache(&state, |db| db.albums_for_artist_name(&name))?;
    Ok(filter_albums(albums, offline_album_source_ids(&state)?))
}

#[tauri::command]
pub async fn get_albums_for_year(state: State<'_, AppState>, year: i32) -> CmdResult<Vec<Album>> {
    let albums = with_cache(&state, |db| db.albums_for_year(year))?;
    Ok(filter_albums(albums, offline_album_source_ids(&state)?))
}

#[tauri::command]
pub async fn get_tracks_for_album(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<Vec<Track>> {
    with_cache(&state, |db| db.tracks_for_album(&source_id))
}

#[tauri::command]
pub async fn get_track(state: State<'_, AppState>, source_id: String) -> CmdResult<Option<Track>> {
    with_cache(&state, |db| db.track_by_source_id(&source_id))
}

#[tauri::command]
pub async fn get_all_artists(state: State<'_, AppState>) -> CmdResult<Vec<ArtistInfo>> {
    let rows = with_cache(&state, |db| db.all_artists())?;
    let mut artists: Vec<ArtistInfo> = rows
        .into_iter()
        .map(|(id, name, source_id, art_url, country)| ArtistInfo {
            id,
            name,
            source_id,
            art_url,
            country,
        })
        .collect();
    if state.effective_offline() {
        let allowed = with_cache(&state, |db| db.downloaded_artist_names())?;
        artists.retain(|a| allowed.contains(&a.name));
    }
    Ok(artists)
}

#[tauri::command]
pub async fn get_filtered_genre_tree(
    state: State<'_, AppState>,
    filters: AlbumFilterParams,
) -> CmdResult<GenreTreeResponse> {
    let mut filtered_ids = with_cache(&state, |db| db.filtered_album_internal_ids(&filters))?;
    if state.effective_offline() {
        let downloaded = with_cache(&state, |db| db.downloaded_album_internal_ids())?;
        filtered_ids = filtered_ids.intersection(&downloaded).copied().collect();
    }
    let all_sets = with_cache(&state, |db| db.genre_album_sets())?;

    let intersected: std::collections::HashMap<String, std::collections::HashSet<i64>> = all_sets
        .into_iter()
        .filter_map(|(genre, ids)| {
            let kept: std::collections::HashSet<i64> =
                ids.intersection(&filtered_ids).copied().collect();
            if kept.is_empty() {
                None
            } else {
                Some((genre, kept))
            }
        })
        .collect();

    let total_album_count = {
        let mut all: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for ids in intersected.values() {
            all.extend(ids);
        }
        all.len()
    };

    let mapper = state.genre_mapper.read();
    let tree = if let Some(mapper) = mapper.as_ref() {
        mapper.build_display_tree(&intersected)
    } else {
        let mut nodes: Vec<GenreNode> = intersected
            .iter()
            .map(|(name, ids)| GenreNode {
                id: name.clone(),
                name: name.clone(),
                short_summary: None,
                children: None,
                album_count: ids.len(),
                deduplicated_total_count: ids.len(),
            })
            .collect();
        nodes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        nodes
    };

    Ok(GenreTreeResponse {
        tree,
        total_album_count,
    })
}

#[tauri::command]
pub async fn toggle_album_favourite(
    state: State<'_, AppState>,
    source_id: String,
    favourite: bool,
) -> CmdResult<()> {
    let api_rating = if favourite { 10.0 } else { 0.0 };
    let db_rating = if favourite { Some(10.0) } else { None };
    state
        .client
        .rate_item(&source_id, api_rating)
        .await
        .map_err(|e| e.to_string())?;
    with_cache(&state, |db| db.update_album_rating(&source_id, db_rating))?;
    Ok(())
}

#[tauri::command]
pub async fn toggle_track_favourite(
    state: State<'_, AppState>,
    source_id: String,
    favourite: bool,
) -> CmdResult<()> {
    let api_rating = if favourite { 10.0 } else { 0.0 };
    let db_rating = if favourite { Some(10.0) } else { None };
    state
        .client
        .rate_item(&source_id, api_rating)
        .await
        .map_err(|e| e.to_string())?;
    with_cache(&state, |db| db.update_track_rating(&source_id, db_rating))?;
    Ok(())
}

#[tauri::command]
pub async fn get_album_genres(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<Vec<String>> {
    with_cache(&state, |db| db.album_genres(&source_id))
}

#[tauri::command]
pub async fn get_album(state: State<'_, AppState>, source_id: String) -> CmdResult<Option<Album>> {
    with_cache(&state, |db| db.album_by_source_id(&source_id))
}

#[tauri::command]
pub async fn get_random_album(state: State<'_, AppState>) -> CmdResult<Option<Album>> {
    if state.effective_offline() {
        let albums = filter_albums(
            with_cache(&state, |db| db.all_albums())?,
            offline_album_source_ids(&state)?,
        );
        if albums.is_empty() {
            return Ok(None);
        }
        let mut rng = rand::thread_rng();
        return Ok(albums.choose(&mut rng).cloned());
    }
    with_cache(&state, |db| db.random_album())
}

#[tauri::command]
pub async fn get_filtered_random_album(
    state: State<'_, AppState>,
    filters: AlbumFilterParams,
) -> CmdResult<Option<Album>> {
    if state.effective_offline() {
        let mut filtered_ids = with_cache(&state, |db| db.filtered_album_internal_ids(&filters))?;
        let downloaded = with_cache(&state, |db| db.downloaded_album_internal_ids())?;
        filtered_ids.retain(|id| downloaded.contains(id));
        if filtered_ids.is_empty() {
            return Ok(None);
        }
        let id_vec: Vec<i64> = filtered_ids.into_iter().collect();
        let mut rng = rand::thread_rng();
        let Some(&chosen) = id_vec.choose(&mut rng) else {
            return Ok(None);
        };
        let albums = with_cache(&state, |db| {
            db.albums_by_internal_ids(&std::collections::HashSet::from([chosen]))
        })?;
        return Ok(albums.into_iter().next());
    }
    with_cache(&state, |db| db.filtered_random_album(&filters))
}

#[tauri::command]
pub async fn get_art_url(
    state: State<'_, AppState>,
    thumb: String,
    size: Option<u32>,
) -> CmdResult<String> {
    let size = size.unwrap_or(300);

    {
        let mut cache = state.image_cache.lock();
        if let Some(path) = cache.get(&thumb, size) {
            return Ok(path.to_string_lossy().to_string());
        }
    }

    // Cache miss: try to download from Plex. On failure (offline, server
    // down, etc), fall back to any smaller size already in the cache —
    // better to render a slightly-fuzzy image than show a placeholder.
    let fetch_result = try_fetch_art(&state, &thumb, size).await;
    match fetch_result {
        Ok(path) => Ok(path),
        Err(e) => {
            if let Some(path) = any_cached_size(&state, &thumb, size) {
                log::debug!(
                    "get_art_url: fetch at {size} failed ({e}); serving alternate cached size"
                );
                Ok(path)
            } else {
                Err(e)
            }
        }
    }
}

async fn try_fetch_art(
    state: &State<'_, AppState>,
    thumb: &str,
    size: u32,
) -> Result<String, String> {
    let server_url = state.client.server_url().ok_or("Not connected")?;
    let token = state.client.token().ok_or("Not authenticated")?;
    let url = plex_art_url(&server_url, thumb, size);
    let response = state
        .http_client
        .get(&url)
        .header("X-Plex-Token", &token)
        .send()
        .await
        .map_err(|_| "Image download failed".to_string())?;
    if !response.status().is_success() {
        return Err(format!("Image download HTTP {}", response.status()));
    }
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    let path = {
        let mut cache = state.image_cache.lock();
        cache
            .insert(thumb, size, &bytes)
            .map_err(|e| e.to_string())?
    };
    Ok(path.to_string_lossy().to_string())
}

/// Walk the canonical art sizes (smaller first, then larger) and return the
/// first one that's already on disk. Used as an offline-grace fallback: a
/// slightly-fuzzy or oversized cached image is always better than rendering
/// a placeholder. Prefers smaller sizes to avoid pushing a 1200px image
/// through a 72px widget, but a larger size still beats nothing.
fn any_cached_size(state: &State<'_, AppState>, thumb: &str, requested: u32) -> Option<String> {
    // Canonical tiers from `ART_SIZE` in ui/src/lib/commands.ts — keep in sync.
    const CANONICAL_SIZES: [u32; 3] = [1200, 300, 72];
    let mut cache = state.image_cache.lock();
    // Smaller first (strictly less than requested).
    for size in CANONICAL_SIZES {
        if size >= requested {
            continue;
        }
        if let Some(path) = cache.get(thumb, size) {
            return Some(path.to_string_lossy().to_string());
        }
    }
    // Nothing smaller — fall back to any larger cached size.
    for size in CANONICAL_SIZES {
        if size <= requested {
            continue;
        }
        if let Some(path) = cache.get(thumb, size) {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}

#[tauri::command]
pub async fn get_album_colors(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<AlbumColorInfo> {
    with_cache(&state, |db| db.album_colors(&source_id))
}

#[tauri::command]
pub async fn set_album_palette(
    state: State<'_, AppState>,
    source_id: String,
    palette: VibrantPalette,
) -> CmdResult<()> {
    with_cache(&state, |db| db.set_album_palette(&source_id, &palette))
}

#[tauri::command]
pub async fn get_cache_stats(state: State<'_, AppState>) -> CmdResult<CacheStats> {
    with_cache(&state, |db| db.cache_stats())
}

#[tauri::command]
pub async fn get_distinct_countries(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    with_cache(&state, |db| db.distinct_artist_countries())
}

#[tauri::command]
pub async fn get_all_collection_names(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    with_cache(&state, |db| db.all_collection_names())
}
