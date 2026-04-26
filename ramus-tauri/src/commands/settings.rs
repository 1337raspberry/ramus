use serde::Serialize;
use tauri::{AppHandle, State};

use ramus_core::genre::mapper::GenreMapper;
use ramus_core::genre::parser::CustomGenreParser;
use ramus_core::models::{SavedSearch, Settings};
use ramus_core::playback::spectrum::spec_file_path;

use crate::events::{emit_connection_status, ConnectionStatusPayload};
use crate::state::AppState;

use super::CmdResult;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCacheStats {
    pub entry_count: usize,
    pub total_size_bytes: u64,
    /// Subset of `entry_count` that is pinned for offline downloads —
    /// these survive `flush_image_cache`.
    pub pinned_count: usize,
    /// Subset of `total_size_bytes` that is pinned for offline downloads.
    pub pinned_size_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioCacheStats {
    pub entry_count: usize,
    pub total_size_bytes: u64,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    Ok(state.settings.read().clone())
}

#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> CmdResult<()> {
    SavedSearch::validate_batch(&settings.saved_searches)?;

    let prev_genre_source = state.settings.read().genre_source;
    let prev_offline_mode = state.settings.read().offline_mode;
    let prev_fuzzy_threshold = state.settings.read().genre_fuzzy_threshold;

    let config = settings.to_playback_config();
    state.player.update_config(config);

    state
        .player
        .apply_equalizer(settings.eq_enabled, &settings.eq_bands);

    state
        .connection_monitor
        .set_allow_http(!settings.refuse_http);

    state
        .image_cache
        .lock()
        .set_limit(settings.image_cache_limit_bytes as u64);

    // Re-emit connection-status if the user toggled Work Offline — same
    // online state, but `effective_offline` flips with the manual flag.
    if settings.offline_mode != prev_offline_mode {
        let online = state
            .server_reachable
            .load(std::sync::atomic::Ordering::Acquire);
        emit_connection_status(
            &app,
            ConnectionStatusPayload {
                online,
                offline_mode_manual: settings.offline_mode,
                effective_offline: settings.offline_mode || !online,
            },
        );
    }

    // Reload genre mapper if source changed.
    if settings.genre_source != prev_genre_source {
        match settings.genre_source {
            ramus_core::models::GenreSource::Custom => {
                if let Some(data) = ramus_core::settings::load_custom_genres() {
                    if let Ok(mapper) = GenreMapper::from_json_bytes(&data) {
                        mapper.set_threshold(settings.genre_fuzzy_threshold);
                        *state.genre_mapper.write() = Some(mapper);
                    }
                }
            }
            ramus_core::models::GenreSource::Open => {
                let open_json = include_bytes!("../../data/open.json");
                if let Ok(mapper) = GenreMapper::from_json_bytes(open_json) {
                    mapper.set_threshold(settings.genre_fuzzy_threshold);
                    *state.genre_mapper.write() = Some(mapper);
                } else {
                    *state.genre_mapper.write() = None;
                }
            }
        }
    } else if (settings.genre_fuzzy_threshold - prev_fuzzy_threshold).abs() > f64::EPSILON {
        // Fuzzy threshold changed without a mapper reload — push to live mapper.
        if let Some(mapper) = state.genre_mapper.read().as_ref() {
            mapper.set_threshold(settings.genre_fuzzy_threshold);
        }
    }

    ramus_core::settings::save(&settings).map_err(|e| e.to_string())?;
    *state.settings.write() = settings;
    Ok(())
}

#[tauri::command]
pub async fn has_custom_genres() -> CmdResult<bool> {
    Ok(ramus_core::settings::load_custom_genres().is_some())
}

#[tauri::command]
pub async fn flush_image_cache(state: State<'_, AppState>) -> CmdResult<()> {
    state.image_cache.lock().flush().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_image_cache_stats(state: State<'_, AppState>) -> CmdResult<ImageCacheStats> {
    let cache = state.image_cache.lock();
    Ok(ImageCacheStats {
        entry_count: cache.entry_count(),
        total_size_bytes: cache.total_size(),
        pinned_count: cache.pinned_count(),
        pinned_size_bytes: cache.pinned_size(),
    })
}

#[tauri::command]
pub async fn import_custom_genres(
    state: State<'_, AppState>,
    text: String,
) -> CmdResult<Vec<String>> {
    let (data, warnings) = CustomGenreParser::parse(&text).map_err(|e| e.to_string())?;
    let mapper = GenreMapper::from_json_bytes(&data).map_err(|e| e.to_string())?;
    ramus_core::settings::save_custom_genres(&data).map_err(|e| e.to_string())?;
    // Persist genre source preference.
    let mut settings = state.settings.read().clone();
    settings.genre_source = ramus_core::models::GenreSource::Custom;
    ramus_core::settings::save(&settings).map_err(|e| e.to_string())?;
    mapper.set_threshold(settings.genre_fuzzy_threshold);
    *state.settings.write() = settings;
    *state.genre_mapper.write() = Some(mapper);
    Ok(warnings)
}

#[tauri::command]
pub async fn remove_custom_genres(state: State<'_, AppState>) -> CmdResult<()> {
    // Delete custom genre file and revert to bundled open.json.
    ramus_core::settings::delete_custom_genres();
    let mut settings = state.settings.read().clone();
    settings.genre_source = ramus_core::models::GenreSource::Open;
    ramus_core::settings::save(&settings).map_err(|e| e.to_string())?;
    *state.settings.write() = settings;
    let open_json = include_bytes!("../../data/open.json");
    if let Ok(mapper) = GenreMapper::from_json_bytes(open_json) {
        mapper.set_threshold(state.settings.read().genre_fuzzy_threshold);
        *state.genre_mapper.write() = Some(mapper);
    } else {
        *state.genre_mapper.write() = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn clear_audio_cache(state: State<'_, AppState>) -> CmdResult<()> {
    state.prefetch_handle.notify_cancel();

    // Clear the in-memory DownloadCache and collect paths to delete.
    let paths = state.player.with_cache(|cache| cache.clear());

    // Delete audio files + sibling .spec files from disk.
    for path in paths {
        let spec = spec_file_path(&path);
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_file(&spec).await;
    }

    Ok(())
}

#[tauri::command]
pub async fn get_audio_cache_stats(state: State<'_, AppState>) -> CmdResult<AudioCacheStats> {
    let (count, size) = state
        .player
        .with_cache(|cache| (cache.len(), cache.total_size()));
    Ok(AudioCacheStats {
        entry_count: count,
        total_size_bytes: size,
    })
}
