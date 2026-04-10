use serde::Serialize;
use tauri::{AppHandle, Emitter};

use ramus_core::cache::sync::SyncProgress;
use ramus_core::models::Track;
use ramus_core::playback::lyrics::LyricsResult;
use ramus_core::playback::mpv::AudioLevels;

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

/// Realtime audio metering from mpv's `astats` filter, in dBFS.
/// Silence is represented by `f64::NEG_INFINITY` (serialised as JSON `null`
/// via a skip, because `serde_json` can't encode `-inf`). The frontend
/// converts dB → linear amplitude when rendering the spectrum visualiser.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioLevelPayload {
    pub left_peak: f64,
    pub right_peak: f64,
    pub left_rms: f64,
    pub right_rms: f64,
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

/// Lowest dB value we forward to the frontend. astats emits `f64::NEG_INFINITY`
/// for silent channels, which serde_json can't encode — we clamp to this
/// instead so JSON is always valid and the frontend sees a normal float.
const MIN_DB: f64 = -120.0;

fn sanitize_db(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        MIN_DB
    }
}

pub fn emit_audio_level(app: &AppHandle, levels: AudioLevels) {
    let payload = AudioLevelPayload {
        left_peak: sanitize_db(levels.left_peak),
        right_peak: sanitize_db(levels.right_peak),
        left_rms: sanitize_db(levels.left_rms),
        right_rms: sanitize_db(levels.right_rms),
    };
    let _ = app.emit("audio-level", payload);
}
