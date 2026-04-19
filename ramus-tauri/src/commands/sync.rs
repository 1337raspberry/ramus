use std::sync::atomic::{AtomicBool, Ordering};
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

/// RAII guard for the `sync_in_progress` atomic flag. The flag is set to
/// `true` on construction and reset to `false` on drop. The spawned background
/// task takes ownership of the guard and forgets it on completion, so the
/// drop path only runs for the early-return error cases on the command thread.
struct SyncGuard(Arc<AtomicBool>);

impl Drop for SyncGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn acquire_sync_guard(flag: &Arc<AtomicBool>) -> CmdResult<SyncGuard> {
    if flag
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("Sync already in progress".into());
    }
    Ok(SyncGuard(flag.clone()))
}

fn get_library_key() -> CmdResult<String> {
    let token_store = TokenStore::new().map_err(|e| e.to_string())?;
    let config = auth::stored_server_config(&token_store).ok_or("No server config")?;
    config
        .selected_library_key
        .ok_or_else(|| "No library selected".into())
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
pub async fn start_full_sync(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let guard = acquire_sync_guard(&state.sync_in_progress)?;

    let library_key = get_library_key()?;
    let engine_lock = state.sync_engine.lock();
    let engine = engine_lock.as_ref().ok_or("Sync engine not initialized")?;

    let cache = engine.cache.clone();
    let client = engine.client.clone();
    drop(engine_lock);

    let sync = ramus_core::cache::sync::SyncEngine::new(cache, client);
    let app_handle = app.clone();
    let settings = state.settings.clone();

    // Hand ownership of the flag to the background task; forgetting the guard
    // prevents its drop from racing the task clearing the flag.
    std::mem::forget(guard);
    let flag = state.sync_in_progress.clone();

    tokio::spawn(async move {
        let result = sync
            .full_sync(&library_key, move |progress| {
                events::emit_sync_progress(&app_handle, progress);
            })
            .await;
        if result.is_ok() {
            update_last_sync_time(&settings);
        }
        flag.store(false, Ordering::Release);
    });

    Ok(())
}

#[tauri::command]
pub async fn start_incremental_sync(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let guard = acquire_sync_guard(&state.sync_in_progress)?;

    let library_key = get_library_key()?;
    let engine_lock = state.sync_engine.lock();
    let engine = engine_lock.as_ref().ok_or("Sync engine not initialized")?;

    let cache = engine.cache.clone();
    let client = engine.client.clone();
    drop(engine_lock);

    let sync = ramus_core::cache::sync::SyncEngine::new(cache, client);
    let app_handle = app.clone();
    let settings = state.settings.clone();

    std::mem::forget(guard);
    let flag = state.sync_in_progress.clone();

    tokio::spawn(async move {
        let result = sync
            .incremental_sync(&library_key, move |progress| {
                events::emit_sync_progress(&app_handle, progress);
            })
            .await;
        if result.is_ok() {
            update_last_sync_time(&settings);
        }
        flag.store(false, Ordering::Release);
    });

    Ok(())
}

#[tauri::command]
pub async fn start_genre_sync(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let guard = acquire_sync_guard(&state.sync_in_progress)?;

    let engine_lock = state.sync_engine.lock();
    let engine = engine_lock.as_ref().ok_or("Sync engine not initialized")?;

    let cache = engine.cache.clone();
    let client = engine.client.clone();
    drop(engine_lock);

    let sync = ramus_core::cache::sync::SyncEngine::new(cache, client);
    let app_handle = app.clone();
    let settings = state.settings.clone();

    std::mem::forget(guard);
    let flag = state.sync_in_progress.clone();

    tokio::spawn(async move {
        let result = sync
            .genre_sync(move |progress| {
                events::emit_sync_progress(&app_handle, progress);
            })
            .await;
        if result.is_ok() {
            update_last_sync_time(&settings);
        }
        flag.store(false, Ordering::Release);
    });

    Ok(())
}
