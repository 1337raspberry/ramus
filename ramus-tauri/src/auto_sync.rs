use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::{Mutex, RwLock};

use ramus_core::cache::sync::SyncEngine;
use ramus_core::models::Settings;

/// Spawns a background task that periodically runs incremental sync
/// based on the `sync_interval_hours` setting.
pub fn spawn(
    settings: Arc<RwLock<Settings>>,
    sync_engine: Arc<Mutex<Option<SyncEngine>>>,
    sync_in_progress: Arc<AtomicBool>,
) {
    tauri::async_runtime::spawn(auto_sync_loop(settings, sync_engine, sync_in_progress));
}

async fn auto_sync_loop(
    settings: Arc<RwLock<Settings>>,
    sync_engine: Arc<Mutex<Option<SyncEngine>>>,
    sync_in_progress: Arc<AtomicBool>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));

    loop {
        interval.tick().await;

        let s = settings.read().clone();
        if s.sync_interval_hours == 0 {
            continue;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let threshold_secs = (s.sync_interval_hours as i64) * 3600;
        let elapsed = now - s.last_sync_time_secs;

        if elapsed < threshold_secs {
            continue;
        }

        // Acquire sync engine lock. Scoped so the guard drops before the await point.
        let (cache, client) = {
            let engine_lock = sync_engine.lock();
            let Some(engine) = engine_lock.as_ref() else {
                continue;
            };
            (engine.cache.clone(), engine.client.clone())
        };

        // Retrieve library key from stored config
        let Ok(token_store) = ramus_core::plex::token_store::TokenStore::new() else {
            continue;
        };
        let Some(config) = ramus_core::plex::auth::stored_server_config(&token_store) else {
            continue;
        };
        let Some(library_key) = config.selected_library_key else {
            continue;
        };

        // Skip if a manual sync is already running
        if sync_in_progress.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            log::info!("auto-sync: skipping, sync already in progress");
            continue;
        }

        let sync = SyncEngine::new(cache, client);

        log::info!(
            "auto-sync: starting incremental sync (interval={}h)",
            s.sync_interval_hours
        );

        let result = sync.incremental_sync(&library_key, |_| {}).await;
        sync_in_progress.store(false, Ordering::Release);

        match result {
            Ok(_) => log::info!("auto-sync: completed successfully"),
            Err(e) => {
                log::warn!("auto-sync: failed: {e}");
                continue;
            }
        }

        // Update last sync time
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let mut s = settings.write();
        s.last_sync_time_secs = now;
        let _ = ramus_core::settings::save(&s);
    }
}
