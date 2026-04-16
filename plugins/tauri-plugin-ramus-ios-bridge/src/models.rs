//! Serde payloads for Rust ↔ Swift plugin calls.
//!
//! All fields use `camelCase` over the wire because the Swift `Invoke`
//! decoder expects JSON that matches Swift property names.

use serde::{Deserialize, Serialize};

/// Used as both request and response type for methods that don't carry
/// a payload in either direction. The custom `Deserialize` swallows
/// whatever Swift sends — `null` from `invoke.resolve()`, `{}` from
/// `invoke.resolve([:])`, or any other shape — without failing. This
/// matters because Tauri's built-in methods like `registerListener`
/// resolve with `null`, and our own methods resolve with `{}`.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Empty {}

impl<'de> Deserialize<'de> for Empty {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        serde::de::IgnoredAny::deserialize(deserializer)?;
        Ok(Empty {})
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadFileArgs {
    pub url: String,
    pub mode: String,
    pub options: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadFileAtArgs {
    pub url: String,
    pub index: i64,
    pub options: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistIndexArgs {
    pub index: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistMoveArgs {
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeekArgs {
    pub position: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseArgs {
    pub paused: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeArgs {
    pub volume: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioFiltersArgs {
    pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeResponse {
    pub volume: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlayingMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub position: f64,
    pub is_playing: bool,
    pub cover_url: Option<String>,
}
