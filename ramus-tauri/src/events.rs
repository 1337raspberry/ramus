use serde::Serialize;
use tauri::{AppHandle, Emitter};

use ramus_core::cache::sync::SyncProgress;
use ramus_core::models::Track;
use ramus_core::playback::lyrics::LyricsResult;

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
pub struct PlaybackBufferingPayload {
    pub is_buffering: bool,
    pub buffered_fraction: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaveformDataPayload {
    pub levels: Vec<f32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionChangedPayload {
    pub server_url: String,
    pub is_local: bool,
    pub is_http: bool,
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

pub fn emit_playback_buffering(app: &AppHandle, payload: PlaybackBufferingPayload) {
    let _ = app.emit("playback-buffering", payload);
}

pub fn emit_waveform_data(app: &AppHandle, payload: WaveformDataPayload) {
    let _ = app.emit("waveform-data", payload);
}

pub fn emit_sync_progress(app: &AppHandle, progress: SyncProgress) {
    let _ = app.emit("sync-progress", progress);
}

pub fn emit_connection_changed(app: &AppHandle, payload: ConnectionChangedPayload) {
    let _ = app.emit("connection-changed", payload);
}

pub fn emit_connection_lost(app: &AppHandle) {
    let _ = app.emit("connection-lost", ());
}

pub fn emit_accent_color(app: &AppHandle, payload: AccentColorPayload) {
    let _ = app.emit("accent-color", payload);
}

pub fn emit_lyrics_update(app: &AppHandle, lyrics: LyricsResult) {
    let _ = app.emit("lyrics-update", lyrics);
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
