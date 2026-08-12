//! Plex playlist commands. Playlists are per-user server objects; every
//! mutation goes to the server first and the local mirror is updated only
//! after a 2xx (same policy as collections). Reads try the server (lazy
//! fetch-on-view keeps the mirror fresh without a sync phase) and fall back
//! to the mirror when unreachable.

use tauri::State;

use ramus_core::cache::playlist::{PlaylistItemRow, PlaylistUpsertRow};
use ramus_core::models::{FilterChoice, Playlist, PlaylistItem};
use ramus_core::plex::auth;
use ramus_core::plex::client::build_library_uri;
use ramus_core::plex::client::PlaylistMetadata;
use ramus_core::plex::smart::{build_smart_uri, SmartFilter};
use ramus_core::plex::token_store::TokenStore;

use crate::state::AppState;

use super::sync::get_library_key;
use super::{with_cache, CmdResult};

fn get_machine_identifier() -> CmdResult<String> {
    let token_store = TokenStore::new().map_err(|e| e.to_string())?;
    let config = auth::stored_server_config(&token_store).ok_or("No server config")?;
    Ok(config.machine_identifier)
}

fn to_upsert_row(m: &PlaylistMetadata) -> PlaylistUpsertRow {
    PlaylistUpsertRow {
        source_id: m.rating_key.clone(),
        title: m.title.clone(),
        smart: m.smart,
        track_count: m.leaf_count,
        duration_ms: m.duration,
        thumb: m.composite.clone(),
    }
}

/// Refuse mutations against a smart playlist — its item list is
/// filter-driven server-side, so the item endpoints don't apply.
fn ensure_not_smart(state: &State<'_, AppState>, source_id: &str) -> CmdResult<()> {
    let playlists = with_cache(state, |db| db.all_playlists())?;
    if playlists.iter().any(|p| p.source_id == source_id && p.smart) {
        return Err("Smart playlists can't be edited".into());
    }
    Ok(())
}

/// Fetch a playlist's entries from the server, mirror them, and return the
/// mirrored (library-joined) view.
async fn refresh_items(
    state: &State<'_, AppState>,
    source_id: &str,
) -> CmdResult<Vec<PlaylistItem>> {
    let fetched = state
        .client
        .playlist_items(source_id)
        .await
        .map_err(|e| e.to_string())?;
    let rows: Vec<PlaylistItemRow> = fetched
        .iter()
        .map(|m| PlaylistItemRow {
            plex_item_id: m.playlist_item_id,
            track_source_id: m.rating_key.clone(),
        })
        .collect();
    with_cache(state, |db| db.replace_playlist_items(source_id, &rows))?;
    with_cache(state, |db| db.playlist_items(source_id))
}

/// List audio playlists. Server first (mirroring the result), local mirror
/// when offline.
#[tauri::command]
pub async fn get_playlists(state: State<'_, AppState>) -> CmdResult<Vec<Playlist>> {
    match state.client.list_audio_playlists().await {
        Ok(list) => {
            let rows: Vec<PlaylistUpsertRow> = list.iter().map(to_upsert_row).collect();
            with_cache(&state, |db| db.replace_playlists(&rows))?;
            with_cache(&state, |db| db.all_playlists())
        }
        Err(_) => with_cache(&state, |db| db.all_playlists()),
    }
}

/// A playlist's entries in order. Server first, mirror fallback.
#[tauri::command]
pub async fn get_playlist_items(
    state: State<'_, AppState>,
    source_id: String,
) -> CmdResult<Vec<PlaylistItem>> {
    match refresh_items(&state, &source_id).await {
        Ok(items) => Ok(items),
        Err(_) => with_cache(&state, |db| db.playlist_items(&source_id)),
    }
}

/// Create a regular playlist from the given track ratingKeys.
#[tauri::command]
pub async fn create_playlist(
    state: State<'_, AppState>,
    title: String,
    track_ids: Vec<String>,
) -> CmdResult<Playlist> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("Playlist name is empty".into());
    }
    if track_ids.is_empty() {
        return Err("Playlist needs at least one track".into());
    }
    let machine_id = get_machine_identifier()?;
    let uri = build_library_uri(&machine_id, &track_ids);
    let created = state
        .client
        .create_playlist(&title, &uri)
        .await
        .map_err(|e| e.to_string())?;
    with_cache(&state, |db| db.upsert_playlist(&to_upsert_row(&created)))?;
    // Best-effort: mirror the entries too, so the detail view opens warm.
    let _ = refresh_items(&state, &created.rating_key).await;
    Ok(Playlist {
        source_id: created.rating_key.clone(),
        title: created.title.clone(),
        smart: created.smart,
        track_count: created.leaf_count,
        duration: created.duration.map(|ms| ms as f64 / 1000.0),
        thumb: created.composite.clone(),
    })
}

/// Create a smart playlist from a filter. The server stores the query and
/// recomputes the track list on every read. Zero rules are allowed only
/// together with a limit — otherwise the "filter" is the entire library.
#[tauri::command]
pub async fn create_smart_playlist(
    state: State<'_, AppState>,
    title: String,
    filter: SmartFilter,
) -> CmdResult<Playlist> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("Playlist name is empty".into());
    }
    if filter
        .terms
        .iter()
        .any(|t| t.field.trim().is_empty() || t.values.is_empty())
    {
        return Err("Malformed filter rule".into());
    }
    if filter.terms.is_empty() && filter.limit.is_none() {
        return Err("Add at least one rule or a track limit".into());
    }
    let machine_id = get_machine_identifier()?;
    let section_key = get_library_key()?;
    let uri = build_smart_uri(&machine_id, &section_key, &filter);
    let created = state
        .client
        .create_smart_playlist(&title, &uri)
        .await
        .map_err(|e| e.to_string())?;
    // Force the smart flag rather than trusting the response's shape — a
    // missed flag would leave the mirror thinking this playlist takes item
    // edits (`ensure_not_smart` reads the mirror).
    let mut row = to_upsert_row(&created);
    row.smart = true;
    with_cache(&state, |db| db.upsert_playlist(&row))?;
    // Best-effort: mirror the computed entries too, so the detail view
    // opens warm.
    let _ = refresh_items(&state, &created.rating_key).await;
    Ok(Playlist {
        source_id: created.rating_key.clone(),
        title: created.title.clone(),
        smart: true,
        track_count: created.leaf_count,
        duration: created.duration.map(|ms| ms as f64 / 1000.0),
        thumb: created.composite.clone(),
    })
}

/// The selectable values for a smart-filter tag field. Curated allowlist —
/// the server silently ignores unknown filter fields (returning the whole
/// library), so field names are never passed through raw.
#[tauri::command]
pub async fn get_smart_filter_choices(
    state: State<'_, AppState>,
    field: String,
) -> CmdResult<Vec<FilterChoice>> {
    // Genres are album-scoped (the whole genre model is album-level).
    let (path_field, type_code) = match field.as_str() {
        "album.genre" => ("genre", 9),
        _ => return Err("Unknown filter field".into()),
    };
    let section_key = get_library_key()?;
    state
        .client
        .filter_choices(&section_key, path_field, type_code)
        .await
        .map_err(|e| e.to_string())
}

/// Append tracks to a playlist; returns the refreshed entry list.
#[tauri::command]
pub async fn add_tracks_to_playlist(
    state: State<'_, AppState>,
    source_id: String,
    track_ids: Vec<String>,
) -> CmdResult<Vec<PlaylistItem>> {
    if track_ids.is_empty() {
        return Err("No tracks given".into());
    }
    ensure_not_smart(&state, &source_id)?;
    let machine_id = get_machine_identifier()?;
    let uri = build_library_uri(&machine_id, &track_ids);
    state
        .client
        .add_playlist_items(&source_id, &uri)
        .await
        .map_err(|e| e.to_string())?;
    refresh_items(&state, &source_id).await
}

/// Remove one entry (by per-item id); returns the refreshed entry list.
#[tauri::command]
pub async fn remove_playlist_item(
    state: State<'_, AppState>,
    source_id: String,
    playlist_item_id: i64,
) -> CmdResult<Vec<PlaylistItem>> {
    ensure_not_smart(&state, &source_id)?;
    state
        .client
        .remove_playlist_item(&source_id, playlist_item_id)
        .await
        .map_err(|e| e.to_string())?;
    refresh_items(&state, &source_id).await
}

/// Move one entry after another (`after_item_id: None` = to the front);
/// returns the refreshed entry list.
#[tauri::command]
pub async fn move_playlist_item(
    state: State<'_, AppState>,
    source_id: String,
    playlist_item_id: i64,
    after_item_id: Option<i64>,
) -> CmdResult<Vec<PlaylistItem>> {
    ensure_not_smart(&state, &source_id)?;
    state
        .client
        .move_playlist_item(&source_id, playlist_item_id, after_item_id)
        .await
        .map_err(|e| e.to_string())?;
    refresh_items(&state, &source_id).await
}

/// Rename a playlist; returns the updated playlist. Deliberately not gated
/// by `ensure_not_smart` — the title lives on the playlist object itself,
/// so renaming a smart playlist is fine (its filter is untouched).
#[tauri::command]
pub async fn rename_playlist(
    state: State<'_, AppState>,
    source_id: String,
    title: String,
) -> CmdResult<Playlist> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("Playlist name is empty".into());
    }
    state
        .client
        .rename_playlist(&source_id, &title)
        .await
        .map_err(|e| e.to_string())?;
    with_cache(&state, |db| db.rename_playlist(&source_id, &title))?;
    let playlists = with_cache(&state, |db| db.all_playlists())?;
    playlists
        .into_iter()
        .find(|p| p.source_id == source_id)
        .ok_or_else(|| "Playlist not found".to_string())
}

/// Delete a playlist server-side and drop it from the mirror.
#[tauri::command]
pub async fn delete_playlist(state: State<'_, AppState>, source_id: String) -> CmdResult<()> {
    state
        .client
        .delete_playlist(&source_id)
        .await
        .map_err(|e| e.to_string())?;
    with_cache(&state, |db| db.remove_playlist(&source_id))
}
