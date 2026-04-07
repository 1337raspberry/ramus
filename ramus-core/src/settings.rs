use std::path::PathBuf;

use crate::models::Settings;
use crate::plex::token_store;

const SETTINGS_FILE: &str = "settings.json";

fn settings_path() -> Option<PathBuf> {
    token_store::config_dir().ok().map(|d| d.join(SETTINGS_FILE))
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
