use tauri::State;

use ramus_core::cache::db::CacheStats;
use ramus_core::genre::node::GenreNode;
use ramus_core::models::{Album, ArtistInfo, Track};

use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

fn with_cache<F, T>(state: &AppState, f: F) -> CmdResult<T>
where
    F: FnOnce(&ramus_core::cache::db::CacheDatabase) -> Result<T, ramus_core::cache::db::CacheError>,
{
    let lock = state.cache.lock();
    let db = lock.as_ref().ok_or("Cache not initialized")?;
    f(db).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_genre_tree(state: State<'_, AppState>) -> CmdResult<Vec<GenreNode>> {
    let mapper = state.genre_mapper.read();
    let mapper = mapper.as_ref().ok_or("Genre mapper not loaded")?;
    let genre_album_sets = with_cache(&state, |db| db.genre_album_sets())?;
    Ok(mapper.build_display_tree(&genre_album_sets))
}

#[tauri::command]
pub async fn get_albums_for_genre(
    state: State<'_, AppState>,
    genre: String,
) -> CmdResult<Vec<Album>> {
    with_cache(&state, |db| db.albums_for_genre(&genre))
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
pub async fn get_albums_for_artist(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<Vec<Album>> {
    with_cache(&state, |db| db.albums_for_artist(&source_id))
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
pub async fn get_favourite_genre_tree(state: State<'_, AppState>) -> CmdResult<Vec<GenreNode>> {
    let mapper = state.genre_mapper.read();
    let mapper = mapper.as_ref().ok_or("Genre mapper not loaded")?;

    // Get favourite album IDs, then build genre sets restricted to favourites
    let fav_ids = with_cache(&state, |db| db.album_ids_for_favourites())?;
    let all_sets = with_cache(&state, |db| db.genre_album_sets())?;

    // Filter genre album sets to only include favourite album IDs
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

    Ok(mapper.build_display_tree(&filtered))
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
        .map_err(|e| e.to_string())
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
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_album_genres(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<Vec<String>> {
    with_cache(&state, |db| db.album_genres(&source_id))
}

#[tauri::command]
pub async fn get_random_album(state: State<'_, AppState>) -> CmdResult<Option<Album>> {
    let albums = with_cache(&state, |db| db.all_albums())?;
    if albums.is_empty() {
        return Ok(None);
    }
    use rand::Rng;
    let idx = rand::thread_rng().gen_range(0..albums.len());
    Ok(Some(albums[idx].clone()))
}

#[tauri::command]
pub async fn get_art_url(
    state: State<'_, AppState>,
    thumb: String,
    size: Option<u32>,
) -> CmdResult<String> {
    let server_url = state.client.server_url().ok_or("Not connected")?;
    let token = state.client.token().ok_or("Not authenticated")?;
    let size = size.unwrap_or(300);
    Ok(format!(
        "{}/photo/:/transcode?width={}&height={}&minSize=1&upscale=1&url={}&X-Plex-Token={}",
        server_url.as_str().trim_end_matches('/'),
        size,
        size,
        urlencoding::encode(&thumb),
        token,
    ))
}

#[tauri::command]
pub async fn get_cache_stats(state: State<'_, AppState>) -> CmdResult<CacheStats> {
    with_cache(&state, |db| db.cache_stats())
}
