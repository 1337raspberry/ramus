use serde::Serialize;
use tauri::State;

use ramus_core::genre::mapper::GenreMapper;
use ramus_core::genre::parser::CustomGenreParser;
use ramus_core::models::Settings;

use crate::state::AppState;

use super::CmdResult;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCacheStats {
    pub entry_count: usize,
    pub total_size_bytes: u64,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    Ok(state.settings.read().clone())
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> CmdResult<()> {
    let prev_genre_source = state.settings.read().genre_source.clone();

    // Update player config if playback settings changed
    let config = settings.to_playback_config();
    state.player.update_config(config);

    // Update connection monitor HTTP policy
    state.connection_monitor.set_allow_http(!settings.refuse_http);

    // Update image cache limit
    state
        .image_cache
        .lock()
        .set_limit(settings.image_cache_limit_bytes as u64);

    // Reload genre mapper if genre source changed
    if settings.genre_source != prev_genre_source {
        match settings.genre_source {
            ramus_core::models::GenreSource::Custom => {
                if let Some(data) = ramus_core::settings::load_custom_genres() {
                    if let Ok(mapper) = GenreMapper::from_json_bytes(&data) {
                        *state.genre_mapper.write() = Some(mapper);
                    }
                }
            }
            ramus_core::models::GenreSource::Open => {
                let open_json = include_bytes!("../../data/open.json");
                if let Ok(mapper) = GenreMapper::from_json_bytes(open_json) {
                    *state.genre_mapper.write() = Some(mapper);
                } else {
                    *state.genre_mapper.write() = None;
                }
            }
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
    })
}

#[tauri::command]
pub async fn import_custom_genres(
    state: State<'_, AppState>,
    text: String,
) -> CmdResult<Vec<String>> {
    let (data, warnings) = CustomGenreParser::parse(&text).map_err(|e| e.to_string())?;
    let mapper = GenreMapper::from_json_bytes(&data).map_err(|e| e.to_string())?;
    // Persist the custom genre JSON to disk
    ramus_core::settings::save_custom_genres(&data).map_err(|e| e.to_string())?;
    // Update settings to remember genre source
    let mut settings = state.settings.read().clone();
    settings.genre_source = ramus_core::models::GenreSource::Custom;
    ramus_core::settings::save(&settings).map_err(|e| e.to_string())?;
    *state.settings.write() = settings;
    *state.genre_mapper.write() = Some(mapper);
    Ok(warnings)
}

#[tauri::command]
pub async fn remove_custom_genres(state: State<'_, AppState>) -> CmdResult<()> {
    // Delete custom genre file and reset to bundled open.json
    ramus_core::settings::delete_custom_genres();
    let mut settings = state.settings.read().clone();
    settings.genre_source = ramus_core::models::GenreSource::Open;
    ramus_core::settings::save(&settings).map_err(|e| e.to_string())?;
    *state.settings.write() = settings;
    let open_json = include_bytes!("../../data/open.json");
    if let Ok(mapper) = GenreMapper::from_json_bytes(open_json) {
        *state.genre_mapper.write() = Some(mapper);
    } else {
        *state.genre_mapper.write() = None;
    }
    Ok(())
}
