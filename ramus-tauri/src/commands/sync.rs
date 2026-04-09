use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use tauri::{AppHandle, State};

use ramus_core::models::Settings;
use ramus_core::plex::auth;
use ramus_core::plex::token_store::TokenStore;

use crate::events;
use crate::state::AppState;

use super::CmdResult;

fn get_library_key() -> CmdResult<String> {
    let token_store = TokenStore::new().map_err(|e| e.to_string())?;
    let config = auth::stored_server_config(&token_store).ok_or("No server config")?;
    config.selected_library_key.ok_or_else(|| "No library selected".into())
}

fn update_last_sync_time(settings: &Arc<RwLock<Settings>>) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut s = settings.write();
    s.last_sync_time_secs = now;
    let _ = ramus_core::settings::save(&s);
}

#[tauri::command]
pub async fn start_full_sync(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<()> {
    let library_key = get_library_key()?;
    let engine_lock = state.sync_engine.lock();
    let engine = engine_lock.as_ref().ok_or("Sync engine not initialized")?;

    // Clone internals needed for the spawned task
    let cache = engine.cache.clone();
    let client = engine.client.clone();
    drop(engine_lock);

    let sync = ramus_core::cache::sync::SyncEngine::new(cache, client);
    let app_handle = app.clone();
    let settings = state.settings.clone();

    tokio::spawn(async move {
        let _ = sync
            .full_sync(&library_key, move |progress| {
                events::emit_sync_progress(&app_handle, progress);
            })
            .await;
        update_last_sync_time(&settings);
    });

    Ok(())
}

#[tauri::command]
pub async fn start_incremental_sync(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<()> {
    let library_key = get_library_key()?;
    let engine_lock = state.sync_engine.lock();
    let engine = engine_lock.as_ref().ok_or("Sync engine not initialized")?;

    let cache = engine.cache.clone();
    let client = engine.client.clone();
    drop(engine_lock);

    let sync = ramus_core::cache::sync::SyncEngine::new(cache, client);
    let app_handle = app.clone();
    let settings = state.settings.clone();

    tokio::spawn(async move {
        let _ = sync
            .incremental_sync(&library_key, move |progress| {
                events::emit_sync_progress(&app_handle, progress);
            })
            .await;
        update_last_sync_time(&settings);
    });

    Ok(())
}

#[tauri::command]
pub async fn start_genre_sync(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<()> {
    let engine_lock = state.sync_engine.lock();
    let engine = engine_lock.as_ref().ok_or("Sync engine not initialized")?;

    let cache = engine.cache.clone();
    let client = engine.client.clone();
    drop(engine_lock);

    let sync = ramus_core::cache::sync::SyncEngine::new(cache, client);
    let app_handle = app.clone();
    let settings = state.settings.clone();

    tokio::spawn(async move {
        let _ = sync
            .genre_sync(move |progress| {
                events::emit_sync_progress(&app_handle, progress);
            })
            .await;
        update_last_sync_time(&settings);
    });

    Ok(())
}
