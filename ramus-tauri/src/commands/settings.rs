use tauri::State;

use ramus_core::genre::mapper::GenreMapper;
use ramus_core::genre::parser::CustomGenreParser;
use ramus_core::models::Settings;

use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    Ok(state.settings.read().clone())
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> CmdResult<()> {
    // Update player config if playback settings changed
    let config = settings.to_playback_config();
    state.player.update_config(config);

    // Update connection monitor HTTP policy
    state.connection_monitor.set_allow_http(!settings.refuse_http);

    ramus_core::settings::save(&settings).map_err(|e| e.to_string())?;
    *state.settings.write() = settings;
    Ok(())
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
