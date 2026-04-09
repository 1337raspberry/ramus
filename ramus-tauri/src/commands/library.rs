use tauri::State;

use ramus_core::cache::db::CacheStats;
use ramus_core::genre::node::GenreNode;
use ramus_core::models::{Album, AlbumColorInfo, ArtistInfo, Track, VibrantPalette};
use ramus_core::search::engine::GenreExpander;
use serde::Serialize;

use crate::state::AppState;

use super::CmdResult;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreTreeResponse {
    pub tree: Vec<GenreNode>,
    pub total_album_count: usize,
}

fn with_cache<F, T>(state: &AppState, f: F) -> CmdResult<T>
where
    F: FnOnce(&ramus_core::cache::db::CacheDatabase) -> Result<T, ramus_core::cache::db::CacheError>,
{
    let lock = state.cache.lock();
    let db = lock.as_ref().ok_or("Cache not initialized")?;
    f(db).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_genre_tree(state: State<'_, AppState>) -> CmdResult<GenreTreeResponse> {
    let genre_album_sets = with_cache(&state, |db| db.genre_album_sets())?;

    // Deduplicated total: union of all album IDs across genres
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
        // Fallback: flat genre list when mapper is not loaded
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

    Ok(GenreTreeResponse { tree, total_album_count })
}

#[tauri::command]
pub async fn get_albums_for_genre(
    state: State<'_, AppState>,
    genre: String,
) -> CmdResult<Vec<Album>> {
    // Expand parent genre to include all descendant genres
    let mapper = state.genre_mapper.read();
    let names: Vec<String> = if let Some(mapper) = mapper.as_ref() {
        if let Some(expanded) = mapper.expand_genre(&genre) {
            expanded.into_iter().collect()
        } else {
            vec![genre.clone()]
        }
    } else {
        vec![genre.clone()]
    };
    drop(mapper);

    let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    with_cache(&state, |db| db.albums_for_genres(&name_refs))
}

#[tauri::command]
pub async fn get_all_albums(state: State<'_, AppState>) -> CmdResult<Vec<Album>> {
    with_cache(&state, |db| db.all_albums())
}

#[tauri::command]
pub async fn get_favourite_albums(state: State<'_, AppState>) -> CmdResult<Vec<Album>> {
    with_cache(&state, |db| db.favourite_albums())
}

#[tauri::command]
pub async fn get_favourite_tracks(state: State<'_, AppState>) -> CmdResult<Vec<Track>> {
    with_cache(&state, |db| db.favourite_tracks())
}

#[tauri::command]
pub async fn get_albums_for_artist(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<Vec<Album>> {
    with_cache(&state, |db| db.albums_for_artist(&source_id))
}

#[tauri::command]
pub async fn get_albums_for_artist_name(
    state: State<'_, AppState>,
    name: String,
) -> CmdResult<Vec<Album>> {
    with_cache(&state, |db| db.albums_for_artist_name(&name))
}

#[tauri::command]
pub async fn get_albums_for_year(
    state: State<'_, AppState>,
    year: i32,
) -> CmdResult<Vec<Album>> {
    with_cache(&state, |db| db.albums_for_year(year))
}

#[tauri::command]
pub async fn get_tracks_for_album(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<Vec<Track>> {
    with_cache(&state, |db| db.tracks_for_album(&source_id))
}

#[tauri::command]
pub async fn get_all_artists(state: State<'_, AppState>) -> CmdResult<Vec<ArtistInfo>> {
    let rows = with_cache(&state, |db| db.all_artists())?;
    Ok(rows
        .into_iter()
        .map(|(id, name, source_id, art_url)| ArtistInfo {
            id,
            name,
            source_id,
            art_url,
        })
        .collect())
}

#[tauri::command]
pub async fn get_favourite_genre_tree(state: State<'_, AppState>) -> CmdResult<GenreTreeResponse> {
    // Build genre sets restricted to favourite albums
    let fav_ids = with_cache(&state, |db| db.album_ids_for_favourites())?;
    let all_sets = with_cache(&state, |db| db.genre_album_sets())?;

    // Intersect genre album sets with favourite IDs
    let filtered: std::collections::HashMap<String, std::collections::HashSet<i64>> = all_sets
        .into_iter()
        .filter_map(|(genre, ids)| {
            let filtered_ids: std::collections::HashSet<i64> =
                ids.intersection(&fav_ids).copied().collect();
            if filtered_ids.is_empty() {
                None
            } else {
                Some((genre, filtered_ids))
            }
        })
        .collect();

    // Deduplicated total: union of favourite album IDs across genres
    let total_album_count = {
        let mut all: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for ids in filtered.values() {
            all.extend(ids);
        }
        all.len()
    };

    let mapper = state.genre_mapper.read();
    let tree = if let Some(mapper) = mapper.as_ref() {
        mapper.build_display_tree(&filtered)
    } else {
        let mut nodes: Vec<GenreNode> = filtered
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

    Ok(GenreTreeResponse { tree, total_album_count })
}

#[tauri::command]
pub async fn toggle_album_favourite(
    state: State<'_, AppState>,
    source_id: String,
    favourite: bool,
) -> CmdResult<()> {
    let rating = if favourite { 10.0 } else { 0.0 };
    state
        .client
        .rate_item(&source_id, rating)
        .await
        .map_err(|e| e.to_string())?;
    with_cache(&state, |db| db.update_album_rating(&source_id, Some(rating)))?;
    Ok(())
}

#[tauri::command]
pub async fn toggle_track_favourite(
    state: State<'_, AppState>,
    source_id: String,
    favourite: bool,
) -> CmdResult<()> {
    let rating = if favourite { 10.0 } else { 0.0 };
    state
        .client
        .rate_item(&source_id, rating)
        .await
        .map_err(|e| e.to_string())?;
    with_cache(&state, |db| db.update_track_rating(&source_id, Some(rating)))?;
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
pub async fn get_album(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<Option<Album>> {
    with_cache(&state, |db| db.album_by_source_id(&source_id))
}

#[tauri::command]
pub async fn get_random_album(state: State<'_, AppState>) -> CmdResult<Option<Album>> {
    with_cache(&state, |db| db.random_album())
}

#[tauri::command]
pub async fn get_art_url(
    state: State<'_, AppState>,
    thumb: String,
    size: Option<u32>,
) -> CmdResult<String> {
    let size = size.unwrap_or(300);

    // Check disk cache first
    {
        let mut cache = state.image_cache.lock();
        if let Some(path) = cache.get(&thumb, size) {
            return Ok(path.to_string_lossy().to_string());
        }
    }

    // Cache miss; download from Plex
    let server_url = state.client.server_url().ok_or("Not connected")?;
    let token = state.client.token().ok_or("Not authenticated")?;
    let url = format!(
        "{}/photo/:/transcode?width={}&height={}&minSize=1&upscale=1&url={}",
        server_url.as_str().trim_end_matches('/'),
        size,
        size,
        urlencoding::encode(&thumb),
    );

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

    // Store in cache (handles concurrent download race internally)
    let path = {
        let mut cache = state.image_cache.lock();
        cache.insert(&thumb, size, &bytes).map_err(|e| e.to_string())?
    };

    Ok(path.to_string_lossy().to_string())
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
