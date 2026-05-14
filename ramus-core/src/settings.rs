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

/// Load settings from disk, falling back to defaults. New fields get their
/// default values when reading a file written by an older version.
pub fn load() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Settings::default();
    };
    let mut settings: Settings = serde_json::from_slice(&bytes).unwrap_or_default();
    // Pre-libmpv Android builds wrote `eq_bands` sized to the system
    // Equalizer's band count (typically 5). The libmpv build uses a
    // 10-band lavfi chain everywhere; repair the shape on load so the
    // first audio filter the user touches doesn't get five gains piped
    // into a ten-band filter string.
    if settings.eq_bands.len() != 10 {
        settings.eq_bands = vec![0.0; 10];
    }
    settings
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
