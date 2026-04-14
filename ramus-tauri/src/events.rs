use serde::Serialize;
use tauri::{AppHandle, Emitter};

use ramus_core::cache::sync::SyncProgress;
use ramus_core::models::Track;

// --- Event payloads ---

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackStatePayload {
    pub status: String,
    pub current_track: Option<Track>,
    pub queue_index: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPositionPayload {
    pub position: f64,
    pub duration: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccentColorPayload {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

// --- Event emission helpers ---

pub fn emit_playback_state(app: &AppHandle, payload: PlaybackStatePayload) {
    let _ = app.emit("playback-state", payload);
}

pub fn emit_playback_position(app: &AppHandle, payload: PlaybackPositionPayload) {
    let _ = app.emit("playback-position", payload);
}

pub fn emit_sync_progress(app: &AppHandle, progress: SyncProgress) {
    let _ = app.emit("sync-progress", progress);
}

pub fn emit_accent_color(app: &AppHandle, payload: AccentColorPayload) {
    let _ = app.emit("accent-color", payload);
}

/// Payload for `spectrum-ready`: notifies the frontend that a track's
/// spectrogram is now available in the cache. The frontend then invokes
/// `get_spectrum` to pull the actual bytes. We intentionally don't ship
/// the spectrogram in the event itself because Tauri's JSON event bridge
/// is slow for ~1 MB payloads.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrumReadyPayload {
    pub rating_key: String,
}

/// Emit `spectrum-ready` for a freshly analysed track. Safe to call from
/// any thread that holds the `AppHandle` — Tauri's event bus handles the
/// cross-thread fan-out to listeners.
pub fn emit_spectrum_ready(app: &AppHandle, rating_key: impl Into<String>) {
    let payload = SpectrumReadyPayload {
        rating_key: rating_key.into(),
    };
    let _ = app.emit("spectrum-ready", payload);
}
