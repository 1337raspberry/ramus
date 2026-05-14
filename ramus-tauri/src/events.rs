use serde::Serialize;
use tauri::{AppHandle, Emitter};

use ramus_core::cache::sync::SyncProgress;
use ramus_core::models::Track;

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

/// Notifies the frontend that a track's spectrogram is available in the cache.
/// The frontend then invokes `get_spectrum` to fetch the bytes — the payload is
/// kept lightweight because Tauri's JSON event bridge is slow for ~1 MB payloads.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectrumReadyPayload {
    pub rating_key: String,
}

/// Emit `spectrum-ready` for a freshly analysed track. Safe to call from any
/// thread that holds the `AppHandle`.
pub fn emit_spectrum_ready(app: &AppHandle, rating_key: impl Into<String>) {
    let payload = SpectrumReadyPayload {
        rating_key: rating_key.into(),
    };
    let _ = app.emit("spectrum-ready", payload);
}

/// Per-track progress for the Downloads panel. `done` and `failed` are
/// terminal; the frontend uses them to remove the row from the "in progress"
/// list. `bytesWritten` / `totalBytes` update on chunk boundaries.
///
/// `title` / `artistName` / `albumTitle` / `thumb` are denormalised onto
/// the event so the panel doesn't need to call `get_track` 1000 times to
/// render a bulk-download queue.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgressPayload {
    pub rating_key: String,
    pub album_rating_key: String,
    pub title: String,
    pub artist_name: String,
    pub album_title: String,
    pub thumb: Option<String>,
    pub phase: &'static str,
    pub bytes_written: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<String>,
}

pub fn emit_download_progress(app: &AppHandle, payload: DownloadProgressPayload) {
    let _ = app.emit("download-progress", payload);
}

/// Coarse signal that the downloads set changed — the frontend refetches
/// the full overview when it sees one. Separate from `download-progress`
/// (which fires per chunk) so the frontend doesn't rebuild the list 20
/// times a second.
pub fn emit_downloads_changed(app: &AppHandle) {
    let _ = app.emit("downloads-changed", ());
}

/// Server reachability state. Fires on startup and when the `ConnectionMonitor`
/// reports loss or recovery. `effectiveOffline` combines the live
/// reachability check with the user's manual `offlineMode` setting.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatusPayload {
    pub online: bool,
    pub offline_mode_manual: bool,
    pub effective_offline: bool,
}

pub fn emit_connection_status(app: &AppHandle, payload: ConnectionStatusPayload) {
    let _ = app.emit("connection-status", payload);
}

/// Android-only buffering signal — fires `{ buffering: true }` when the
/// Tauri-plugin's transcode pre-download starts, and `{ buffering: false }`
/// when ExoPlayer has the local file and is ready to play. iOS and desktop
/// never emit this. The frontend uses it to drive the scanning-bar overlay
/// on `WaveformSeekBar` without falling back to timing heuristics.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferingStatePayload {
    pub buffering: bool,
}

pub fn emit_buffering_state(app: &AppHandle, payload: BufferingStatePayload) {
    let _ = app.emit("buffering-state", payload);
}
