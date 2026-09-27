//! The active cast, remembered across launches (`cast.json` in the config
//! directory) so a relaunch can pick it back up. Holds no tokens.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::plex::token_store;

const CAST_FILE: &str = "cast.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CastRecord {
    pub player_id: String,
    pub player_name: String,
    pub product: Option<String>,
    /// The player address that answered.
    pub uri: String,
    /// The play queue this cast sent; a relaunch resumes only if the player
    /// still plays it.
    pub play_queue_id: i64,
}

fn path() -> Option<PathBuf> {
    token_store::config_dir().ok().map(|d| d.join(CAST_FILE))
}

pub fn load() -> Option<CastRecord> {
    load_from(&path()?)
}

pub fn save(record: &CastRecord) -> Result<(), String> {
    save_to(&path().ok_or("no config directory available")?, record)
}

pub fn clear() {
    if let Some(path) = path() {
        let _ = std::fs::remove_file(path);
    }
}

fn load_from(path: &Path) -> Option<CastRecord> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn save_to(path: &Path, record: &CastRecord) -> Result<(), String> {
    let json = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    crate::playback::queue_store::write_atomic(path, &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> CastRecord {
        CastRecord {
            player_id: "tv".into(),
            player_name: "Living Room".into(),
            product: Some("ramusTV".into()),
            uri: "http://10.0.0.9:32500/".into(),
            play_queue_id: 9001,
        }
    }

    #[test]
    fn a_record_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CAST_FILE);
        save_to(&path, &record()).unwrap();
        assert_eq!(load_from(&path), Some(record()));
    }

    #[test]
    fn a_missing_or_garbled_file_is_no_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CAST_FILE);
        assert_eq!(load_from(&path), None);
        std::fs::write(&path, b"{not json").unwrap();
        assert_eq!(load_from(&path), None);
    }

    #[test]
    fn the_record_holds_no_token_fields() {
        let json = serde_json::to_string(&record()).unwrap();
        assert!(!json.to_lowercase().contains("token"), "{json}");
    }
}
