use std::path::PathBuf;

use crate::models::Settings;
use crate::plex::token_store;

const SETTINGS_FILE: &str = "settings.json";
const CUSTOM_GENRES_FILE: &str = "custom_genres.json";

fn settings_path() -> Option<PathBuf> {
    token_store::config_dir().ok().map(|d| d.join(SETTINGS_FILE))
}

fn custom_genres_path() -> Option<PathBuf> {
    token_store::config_dir().ok().map(|d| d.join(CUSTOM_GENRES_FILE))
}

/// Load settings from disk, falling back to defaults.
/// Uses `#[serde(default)]` semantics — new fields get their default values
/// even if the file was written by an older version.
pub fn load() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Settings::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Save settings to disk. Creates the config directory if needed.
pub fn save(settings: &Settings) -> Result<(), String> {
    let path = settings_path().ok_or("no config directory available")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

/// Save custom genre JSON data to disk.
pub fn save_custom_genres(data: &[u8]) -> Result<(), String> {
    let path = custom_genres_path().ok_or("no config directory available")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, data).map_err(|e| e.to_string())
}

/// Load custom genre JSON data from disk, if it exists.
pub fn load_custom_genres() -> Option<Vec<u8>> {
    let path = custom_genres_path()?;
    std::fs::read(&path).ok()
}

/// Delete the custom genre file from disk.
pub fn delete_custom_genres() {
    if let Some(path) = custom_genres_path() {
        let _ = std::fs::remove_file(path);
    }
}
