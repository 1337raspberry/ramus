//! AudioPlayer — queue management, mpv integration, equalizer, download cache.
//!
//! The AudioPlayer owns the mpv handle (via `MpvPlayer` trait) and manages
//! the playback queue, track URL resolution, audio download cache (LRU),
//! and 10-band parametric equalizer filter strings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use url::Url;

use crate::models::{PlaybackConfig, PlaybackStatus, PlayerState, Track};
use crate::playback::mpv::{FileEndReason, LoadMode, MpvPlayer};
use crate::playback::transcode;

// --- Constants ---

/// 10-band EQ center frequencies in Hz.
pub const EQ_FREQUENCIES: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

/// Allowed file extensions for cached audio files.
const ALLOWED_EXTENSIONS: &[&str] = &[
    "flac", "alac", "m4a", "mp3", "aac", "wav", "aiff", "ogg", "opus", "mp2", "bin",
];

/// Threshold in seconds: if position > this, `previous()` restarts instead of going back.
const PREVIOUS_RESTART_THRESHOLD: f64 = 3.0;

// --- EQ filter string ---

/// Build an mpv `af` lavfi equalizer filter string from 10 gain values.
///
/// Rust's `format!` always uses '.' for decimals (no locale issues).
/// NaN and Inf values are sanitized to 0.0.
pub fn build_eq_filter_string(bands: &[f32; 10]) -> String {
    let filters: Vec<String> = EQ_FREQUENCIES
        .iter()
        .zip(bands.iter())
        .map(|(freq, gain)| {
            let g = if gain.is_finite() { *gain } else { 0.0 };
            format!("equalizer=f={freq}:width_type=o:w=1:g={g:.1}")
        })
        .collect();

    format!("lavfi=[{}]", filters.join(","))
}

// --- Filename sanitization ---

/// Sanitize a string for use as a filename. Only `[a-zA-Z0-9_-]` are kept.
pub fn sanitize_filename(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

/// Check if a file extension is in the allowed set for audio caching.
pub fn is_allowed_extension(ext: &str) -> bool {
    ALLOWED_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}

// --- DownloadCache (LRU) ---

/// LRU download cache tracking cached audio files.
///
/// Manages metadata only — the caller handles actual file I/O.
/// `evict_if_needed` returns paths to delete; the caller removes them from disk.
pub struct DownloadCache {
    entries: HashMap<String, PathBuf>,
    sizes: HashMap<String, u64>,
    access_order: Vec<String>, // oldest first
    limit_bytes: u64,
}

impl DownloadCache {
    pub fn new(limit_bytes: u64) -> Self {
        Self {
            entries: HashMap::new(),
            sizes: HashMap::new(),
            access_order: Vec::new(),
            limit_bytes,
        }
    }

    /// Get the cached file path for a track, if present.
    pub fn get(&self, track_id: &str) -> Option<&Path> {
        self.entries.get(track_id).map(|p| p.as_path())
    }

    /// Insert a cached file entry.
    pub fn insert(&mut self, track_id: String, path: PathBuf, size: u64) {
        // Remove existing entry if present (update)
        self.access_order.retain(|k| k != &track_id);
        self.entries.insert(track_id.clone(), path);
        self.sizes.insert(track_id.clone(), size);
        self.access_order.push(track_id);
    }

    /// Touch a cache entry, moving it to the most-recently-used position.
    pub fn touch(&mut self, track_id: &str) {
        if self.entries.contains_key(track_id) {
            self.access_order.retain(|k| k != track_id);
            self.access_order.push(track_id.to_string());
        }
    }

    /// Evict oldest entries until total size is within the limit.
    /// Never evicts the currently playing track.
    /// Returns paths that should be deleted from disk.
    pub fn evict_if_needed(&mut self, current_track_id: Option<&str>) -> Vec<PathBuf> {
        let mut evicted = Vec::new();

        while self.total_size() > self.limit_bytes && !self.access_order.is_empty() {
            // Find the oldest entry that isn't the current track
            let idx = self
                .access_order
                .iter()
                .position(|k| current_track_id.is_none_or(|c| k != c));

            if let Some(idx) = idx {
                let key = self.access_order.remove(idx);
                if let Some(path) = self.entries.remove(&key) {
                    evicted.push(path);
                }
                self.sizes.remove(&key);
            } else {
                break; // Only current track left
            }
        }

        evicted
    }

    pub fn total_size(&self) -> u64 {
        self.sizes.values().sum()
    }

    /// Remove a specific entry. Returns the path if it was present.
    pub fn remove(&mut self, track_id: &str) -> Option<PathBuf> {
        self.access_order.retain(|k| k != track_id);
        self.sizes.remove(track_id);
        self.entries.remove(track_id)
    }

    /// Clear all entries. Returns all paths for disk cleanup.
    pub fn clear(&mut self) -> Vec<PathBuf> {
        let paths: Vec<PathBuf> = self.entries.drain().map(|(_, p)| p).collect();
        self.sizes.clear();
        self.access_order.clear();
        paths
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// --- AudioPlayer ---

/// Observable state snapshot for the frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioPlayerState {
    pub state: PlayerState,
    pub position: f64,
    pub duration: f64,
    pub is_loading: bool,
    pub is_buffering: bool,
    pub waveform_levels: Option<Vec<f32>>,
    pub buffered_fraction: f64,
    pub volume: f64,
}

struct PlayerInner {
    state: PlayerState,
    position: f64,
    duration: f64,
    is_loading: bool,
    is_buffering: bool,
    buffered_fraction: f64,
    volume: f64,
    config: PlaybackConfig,
    server_url: Option<Url>,
    token: Option<String>,
    client_identifier: String,
    is_remote: bool,
    play_session_id: String,
    cache: DownloadCache,
}

/// The core audio player managing queue state, mpv commands, and track resolution.
pub struct AudioPlayer {
    mpv: Arc<dyn MpvPlayer>,
    inner: Mutex<PlayerInner>,
}

impl AudioPlayer {
    pub fn new(mpv: Arc<dyn MpvPlayer>) -> Self {
        Self {
            mpv,
            inner: Mutex::new(PlayerInner {
                state: PlayerState::default(),
                position: 0.0,
                duration: 0.0,
                is_loading: false,
                is_buffering: false,
                buffered_fraction: 0.0,
                volume: 50.0,
                config: PlaybackConfig::default(),
                server_url: None,
                token: None,
                client_identifier: String::new(),
                is_remote: false,
                play_session_id: uuid::Uuid::new_v4().to_string(),
                cache: DownloadCache::new(PlaybackConfig::DEFAULT_CACHE_LIMIT_BYTES as u64),
            }),
        }
    }

    // --- Configuration ---

    /// Set server connection details.
    pub fn configure(&self, server_url: Url, token: String, client_identifier: String) {
        let mut inner = self.inner.lock();
        inner.server_url = Some(server_url);
        inner.token = Some(token);
        inner.client_identifier = client_identifier;
    }

    /// Update server connection (e.g., after failover or reconnection).
    pub fn update_server_connection(&self, server_url: Url, token: String, is_remote: bool) {
        let mut inner = self.inner.lock();
        inner.server_url = Some(server_url);
        inner.token = Some(token);
        inner.is_remote = is_remote;
    }

    /// Update playback configuration.
    pub fn update_config(&self, config: PlaybackConfig) {
        let mut inner = self.inner.lock();
        inner.cache.limit_bytes = config.audio_cache_limit_bytes as u64;
        inner.config = config;
    }

    // --- Queue operations ---

    /// Replace the queue and start playback at the given index.
    ///
    /// Uses `loadfile "replace"` for the first track and `"append"` for the rest.
    /// Does NOT call `mpv.stop()` first — `replace` handles that implicitly.
    pub fn load_queue(&self, tracks: Vec<Track>, start_at: usize) {
        if tracks.is_empty() || start_at >= tracks.len() {
            return;
        }

        let urls = {
            let mut inner = self.inner.lock();
            inner.state.queue = tracks;
            inner.state.queue_index = start_at;
            inner.state.current_track = Some(inner.state.queue[start_at].clone());
            inner.state.status = PlaybackStatus::Playing;
            inner.play_session_id = uuid::Uuid::new_v4().to_string();
            inner.position = 0.0;
            inner.duration = 0.0;
            inner.is_loading = false;

            inner
                .state
                .queue
                .iter()
                .map(|t| resolve_url(t, &inner))
                .collect::<Vec<_>>()
        };

        for (i, url) in urls.iter().enumerate() {
            if let Some(url) = url {
                let mode = if i == 0 {
                    LoadMode::Replace
                } else {
                    LoadMode::Append
                };
                self.mpv.load_file(url, mode);
            }
        }

        if start_at > 0 {
            self.mpv.playlist_play_index(start_at as i64);
        }
        self.mpv.set_pause(false);
    }

    /// Append tracks to the end of the queue.
    /// If stopped or queue was empty, starts playback from the beginning.
    pub fn append_to_queue(&self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }

        let (was_stopped, urls) = {
            let mut inner = self.inner.lock();
            let was_stopped =
                inner.state.queue.is_empty() || inner.state.status == PlaybackStatus::Stopped;
            inner.state.queue.extend(tracks.iter().cloned());

            if was_stopped {
                (true, Vec::new())
            } else {
                let urls: Vec<Option<String>> =
                    tracks.iter().map(|t| resolve_url(t, &inner)).collect();
                (false, urls)
            }
        };

        if was_stopped {
            let queue = self.inner.lock().state.queue.clone();
            self.load_queue(queue, 0);
        } else {
            for url in urls.into_iter().flatten() {
                self.mpv.load_file(&url, LoadMode::Append);
            }
        }
    }

    /// Insert tracks immediately after the current track.
    /// If stopped, treats as `load_queue`.
    pub fn insert_next(&self, tracks: Vec<Track>) {
        if tracks.is_empty() {
            return;
        }

        let is_stopped = {
            let inner = self.inner.lock();
            inner.state.status == PlaybackStatus::Stopped
        };

        if is_stopped {
            self.load_queue(tracks, 0);
            return;
        }

        let (insert_base, urls) = {
            let mut inner = self.inner.lock();
            let insert_base = inner.state.queue_index + 1;

            for (offset, track) in tracks.iter().enumerate() {
                inner
                    .state
                    .queue
                    .insert(insert_base + offset, track.clone());
            }

            let urls: Vec<Option<String>> =
                tracks.iter().map(|t| resolve_url(t, &inner)).collect();
            (insert_base, urls)
        };

        for (offset, url) in urls.iter().enumerate() {
            if let Some(url) = url {
                self.mpv
                    .load_file_at(url, (insert_base + offset) as i64);
            }
        }
    }

    /// Remove a track from the queue by index.
    /// Cannot remove the currently playing track. Adjusts queue index if needed.
    pub fn remove_from_queue(&self, index: usize) {
        let mut inner = self.inner.lock();

        if index == inner.state.queue_index {
            return; // Don't remove currently playing
        }
        if index >= inner.state.queue.len() {
            return;
        }

        inner.state.queue.remove(index);
        let mpv_index = index as i64;

        if index < inner.state.queue_index {
            inner.state.queue_index -= 1;
        }

        drop(inner);
        self.mpv.playlist_remove(mpv_index);
    }

    /// Jump to a specific queue position.
    pub fn jump_to_index(&self, index: usize) {
        let mut inner = self.inner.lock();
        if index >= inner.state.queue.len() {
            return;
        }

        inner.state.queue_index = index;
        inner.state.current_track = Some(inner.state.queue[index].clone());
        inner.position = 0.0;
        inner.duration = 0.0;
        inner.state.status = PlaybackStatus::Playing;
        drop(inner);
        self.mpv.playlist_play_index(index as i64);
    }

    /// Advance to the next track. Stops if at the end of the queue.
    pub fn next(&self) {
        let mut inner = self.inner.lock();
        if inner.state.queue_index + 1 >= inner.state.queue.len() {
            inner.state.status = PlaybackStatus::Stopped;
            inner.state.current_track = None;
            inner.position = 0.0;
            drop(inner);
            self.mpv.stop();
            return;
        }

        inner.state.queue_index += 1;
        inner.state.current_track = Some(inner.state.queue[inner.state.queue_index].clone());
        inner.position = 0.0;
        inner.duration = 0.0;
        let idx = inner.state.queue_index;
        drop(inner);
        self.mpv.playlist_play_index(idx as i64);
    }

    /// Go to the previous track, or restart the current track if > 3s in.
    pub fn previous(&self) {
        let mut inner = self.inner.lock();

        if inner.position > PREVIOUS_RESTART_THRESHOLD {
            inner.position = 0.0;
            drop(inner);
            self.mpv.seek(0.0);
            return;
        }

        if inner.state.queue_index == 0 {
            inner.position = 0.0;
            drop(inner);
            self.mpv.seek(0.0);
            return;
        }

        inner.state.queue_index -= 1;
        inner.state.current_track = Some(inner.state.queue[inner.state.queue_index].clone());
        inner.position = 0.0;
        inner.duration = 0.0;
        let idx = inner.state.queue_index;
        drop(inner);
        self.mpv.playlist_play_index(idx as i64);
    }

    /// Toggle between playing and paused.
    pub fn toggle_play_pause(&self) {
        let mut inner = self.inner.lock();
        match inner.state.status {
            PlaybackStatus::Playing => {
                inner.state.status = PlaybackStatus::Paused;
                drop(inner);
                self.mpv.set_pause(true);
            }
            PlaybackStatus::Paused => {
                inner.state.status = PlaybackStatus::Playing;
                drop(inner);
                self.mpv.set_pause(false);
            }
            PlaybackStatus::Stopped => {}
        }
    }

    /// Seek to an absolute position in seconds.
    pub fn seek(&self, position: f64) {
        let mut inner = self.inner.lock();
        let clamped = position.max(0.0).min((inner.duration - 0.5).max(0.0));
        inner.position = clamped;
        drop(inner);
        self.mpv.seek(clamped);
    }

    /// Set playback volume (0–100).
    pub fn set_volume(&self, volume: f64) {
        let clamped = volume.clamp(0.0, 100.0);
        self.inner.lock().volume = clamped;
        self.mpv.set_volume(clamped);
    }

    /// Stop playback and clear the queue.
    pub fn stop(&self) {
        let mut inner = self.inner.lock();
        inner.state.status = PlaybackStatus::Stopped;
        inner.state.current_track = None;
        inner.state.queue.clear();
        inner.state.queue_index = 0;
        inner.position = 0.0;
        inner.duration = 0.0;
        drop(inner);
        self.mpv.stop();
    }

    /// Apply or clear the 10-band equalizer.
    ///
    /// When enabled, builds a lavfi filter string from the gain values.
    /// When disabled, clears filters with "no" (not empty string).
    pub fn apply_equalizer(&self, enabled: bool, bands: &[f32; 10]) {
        if enabled {
            let filter = build_eq_filter_string(bands);
            self.mpv.set_audio_filters(&filter);
        } else {
            self.mpv.set_audio_filters("no");
        }
    }

    // --- State queries ---

    /// Snapshot the full player state for the frontend.
    pub fn snapshot(&self) -> AudioPlayerState {
        let inner = self.inner.lock();
        AudioPlayerState {
            state: inner.state.clone(),
            position: inner.position,
            duration: inner.duration,
            is_loading: inner.is_loading,
            is_buffering: inner.is_buffering,
            waveform_levels: None,
            buffered_fraction: inner.buffered_fraction,
            volume: inner.volume,
        }
    }

    pub fn state(&self) -> PlayerState {
        self.inner.lock().state.clone()
    }

    pub fn position(&self) -> f64 {
        self.inner.lock().position
    }

    pub fn duration(&self) -> f64 {
        self.inner.lock().duration
    }

    pub fn volume(&self) -> f64 {
        self.inner.lock().volume
    }

    pub fn play_session_id(&self) -> String {
        self.inner.lock().play_session_id.clone()
    }

    // --- mpv event handlers ---

    /// Handle mpv position change (called by event loop, ~30fps).
    pub fn handle_position_change(&self, pos: f64) {
        self.inner.lock().position = pos;
    }

    /// Handle mpv duration change.
    pub fn handle_duration_change(&self, dur: f64) {
        self.inner.lock().duration = dur;
    }

    /// Handle mpv playlist-pos change (track advance).
    pub fn handle_playlist_pos_change(&self, pos: i64) {
        if pos < 0 {
            return;
        }
        let mut inner = self.inner.lock();
        let pos = pos as usize;
        if pos >= inner.state.queue.len() {
            return;
        }

        inner.state.queue_index = pos;
        inner.state.current_track = Some(inner.state.queue[pos].clone());
        inner.position = 0.0;
        inner.duration = 0.0;
        inner.buffered_fraction = 0.0;
    }

    /// Handle mpv pause state change.
    pub fn handle_pause_change(&self, paused: bool) {
        let mut inner = self.inner.lock();
        if paused && inner.state.status == PlaybackStatus::Playing {
            inner.state.status = PlaybackStatus::Paused;
        } else if !paused && inner.state.status == PlaybackStatus::Paused {
            inner.state.status = PlaybackStatus::Playing;
        }
    }

    /// Handle mpv buffering state change (paused-for-cache).
    pub fn handle_buffering_change(&self, buffering: bool) {
        self.inner.lock().is_buffering = buffering;
    }

    /// Handle mpv cache-buffering-state change (0–100).
    pub fn handle_cache_state_change(&self, state: i64) {
        self.inner.lock().buffered_fraction = (state as f64 / 100.0).clamp(0.0, 1.0);
    }

    /// Handle mpv file-loaded event.
    pub fn handle_file_loaded(&self) {
        let mut inner = self.inner.lock();
        inner.is_loading = false;
        inner.is_buffering = false;
    }

    /// Handle mpv file-ended event.
    pub fn handle_file_ended(&self, reason: FileEndReason) {
        match reason {
            FileEndReason::Eof => {
                // Natural end — mpv auto-advances via gapless playback.
                // If last track, idle-active will fire.
            }
            FileEndReason::Error(_) => {
                // Skip to next or stop
                self.next();
            }
            _ => {} // Stop, Quit, Redirect — no action
        }
    }

    /// Handle mpv idle-active (queue completed).
    pub fn handle_idle_active(&self) {
        let mut inner = self.inner.lock();
        inner.state.status = PlaybackStatus::Stopped;
        inner.state.current_track = None;
        inner.position = 0.0;
    }

    // --- Cache access (for external download manager) ---

    /// Access the download cache under the player lock.
    pub fn with_cache<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut DownloadCache) -> R,
    {
        let mut inner = self.inner.lock();
        f(&mut inner.cache)
    }


    /// Whether the current track is fully buffered (buffered_fraction >= 1.0).
    pub fn is_fully_buffered(&self) -> bool {
        self.inner.lock().buffered_fraction >= 1.0
    }

    // --- Prefetch ---

    /// Identify upcoming tracks that should be prefetched (downloaded to cache).
    ///
    /// Returns `(rating_key, download_url)` for each track that:
    /// - Is within `lookahead_depth` of the current position
    /// - Is not already in the download cache
    /// - Would use direct play (not HLS transcode)
    pub fn prefetch_targets(&self) -> Vec<(String, String)> {
        let inner = self.inner.lock();
        let depth = inner.config.lookahead_depth as usize;
        let pos = inner.state.queue_index;
        let queue = &inner.state.queue;

        let server_url = match inner.server_url.as_ref() {
            Some(u) => u,
            None => return Vec::new(),
        };
        let token = match inner.token.as_ref() {
            Some(t) => t,
            None => return Vec::new(),
        };

        let mut targets = Vec::new();

        for offset in 1..=depth {
            let idx = pos + offset;
            if idx >= queue.len() {
                break;
            }
            let track = &queue[idx];

            // Skip if already cached
            if inner.cache.get(&track.rating_key).is_some() {
                continue;
            }

            // Skip HLS transcode tracks (streamed on demand)
            if transcode::should_transcode(
                track.codec.as_deref(),
                inner.config.playback_mode,
                inner.is_remote,
            ) {
                continue;
            }

            // Build direct play URL
            if let Some(part_key) = track.part_key.as_ref() {
                if let Some(url) =
                    transcode::build_direct_play_url(server_url, part_key, token)
                {
                    targets.push((track.rating_key.clone(), url.to_string()));
                }
            }
        }

        if !targets.is_empty() {
            log::debug!(
                "prefetch_targets: {} targets, mode={:?}, remote={}",
                targets.len(),
                inner.config.playback_mode,
                inner.is_remote,
            );
        }
        targets
    }

    /// Get the rating_key of the currently playing track (for cache eviction protection).
    pub fn current_track_id(&self) -> Option<String> {
        self.inner
            .lock()
            .state
            .current_track
            .as_ref()
            .map(|t| t.rating_key.clone())
    }
}

// --- URL resolution (free function to avoid lock concerns) ---

fn resolve_url(track: &Track, inner: &PlayerInner) -> Option<String> {
    // Check download cache
    if let Some(path) = inner.cache.get(&track.rating_key) {
        return Some(format!("file://{}", path.display()));
    }

    let server_url = inner.server_url.as_ref()?;
    let token = inner.token.as_ref()?;

    if transcode::should_transcode(
        track.codec.as_deref(),
        inner.config.playback_mode,
        inner.is_remote,
    ) {
        transcode::build_hls_url(
            server_url,
            token,
            &track.rating_key,
            &inner.client_identifier,
            &inner.play_session_id,
        )
        .map(|u| u.to_string())
    } else {
        let part_key = track.part_key.as_ref()?;
        transcode::build_direct_play_url(server_url, part_key, token).map(|u| u.to_string())
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::mpv::MpvPlayer;
    use std::sync::atomic::{AtomicBool, Ordering};

    // --- Mock MpvPlayer ---

    #[derive(Debug, Clone)]
    #[allow(dead_code)] // Fields read via Debug/pattern matching in assertions
    enum MockCall {
        LoadFile { url: String, mode: LoadMode },
        LoadFileAt { url: String, index: i64 },
        PlaylistPlayIndex(i64),
        PlaylistRemove(i64),
        PlaylistMove { from: i64, to: i64 },
        Seek(f64),
        SetPause(bool),
        SetVolume(f64),
        SetAudioFilters(String),
        Stop,
    }

    struct MockMpv {
        calls: Mutex<Vec<MockCall>>,
        volume: Mutex<f64>,
        shutdown: AtomicBool,
    }

    impl MockMpv {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                volume: Mutex::new(50.0),
                shutdown: AtomicBool::new(false),
            }
        }

        fn calls(&self) -> Vec<MockCall> {
            self.calls.lock().clone()
        }

        fn call_count(&self) -> usize {
            self.calls.lock().len()
        }
    }

    impl MpvPlayer for MockMpv {
        fn load_file(&self, url: &str, mode: LoadMode) {
            self.calls.lock().push(MockCall::LoadFile {
                url: url.to_string(),
                mode,
            });
        }
        fn load_file_at(&self, url: &str, index: i64) {
            self.calls.lock().push(MockCall::LoadFileAt {
                url: url.to_string(),
                index,
            });
        }
        fn playlist_play_index(&self, index: i64) {
            self.calls.lock().push(MockCall::PlaylistPlayIndex(index));
        }
        fn playlist_remove(&self, index: i64) {
            self.calls.lock().push(MockCall::PlaylistRemove(index));
        }
        fn playlist_move(&self, from: i64, to: i64) {
            self.calls.lock().push(MockCall::PlaylistMove { from, to });
        }
        fn seek(&self, position: f64) {
            self.calls.lock().push(MockCall::Seek(position));
        }
        fn set_pause(&self, paused: bool) {
            self.calls.lock().push(MockCall::SetPause(paused));
        }
        fn set_volume(&self, volume: f64) {
            *self.volume.lock() = volume;
            self.calls.lock().push(MockCall::SetVolume(volume));
        }
        fn get_volume(&self) -> f64 {
            *self.volume.lock()
        }
        fn set_audio_filters(&self, value: &str) {
            self.calls
                .lock()
                .push(MockCall::SetAudioFilters(value.to_string()));
        }
        fn stop(&self) {
            self.calls.lock().push(MockCall::Stop);
        }
        fn is_shutdown(&self) -> bool {
            self.shutdown.load(Ordering::Acquire)
        }
    }

    // --- Test helpers ---

    fn make_test_track(key: &str) -> Track {
        Track {
            rating_key: key.into(),
            title: format!("Track {key}"),
            artist_name: "Test Artist".into(),
            track_artist: None,
            album_title: "Test Album".into(),
            album_key: None,
            index: None,
            duration: 180.0,
            codec: Some("flac".into()),
            part_key: Some(format!("/library/parts/{key}/file.flac")),
            thumb: None,
            is_favourite: false,
            bitrate: None,
            disc_number: None,
        }
    }

    fn make_player() -> (AudioPlayer, Arc<MockMpv>) {
        let mpv = Arc::new(MockMpv::new());
        let player = AudioPlayer::new(mpv.clone());
        player.configure(
            Url::parse("http://test.local:32400").unwrap(),
            "test-token".into(),
            "test-client".into(),
        );
        (player, mpv)
    }

    // --- EQ filter string tests ---

    #[test]
    fn test_eq_filter_string_all_zeros() {
        let bands = [0.0f32; 10];
        let filter = build_eq_filter_string(&bands);
        assert!(filter.starts_with("lavfi=["));
        assert!(filter.ends_with(']'));
        assert!(filter.contains("equalizer=f=31:width_type=o:w=1:g=0.0"));
        assert!(filter.contains("equalizer=f=16000:width_type=o:w=1:g=0.0"));
        // Should have 10 equalizer sections
        assert_eq!(filter.matches("equalizer=").count(), 10);
    }

    #[test]
    fn test_eq_filter_string_with_gains() {
        let bands = [3.5, -2.0, 0.0, 1.0, -1.5, 6.0, -12.0, 12.0, 0.5, -0.5];
        let filter = build_eq_filter_string(&bands);
        assert!(filter.contains("g=3.5"));
        assert!(filter.contains("g=-2.0"));
        assert!(filter.contains("g=6.0"));
        assert!(filter.contains("g=-12.0"));
        assert!(filter.contains("g=12.0"));
    }

    #[test]
    fn test_eq_filter_string_decimal_point_not_comma() {
        let bands = [3.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let filter = build_eq_filter_string(&bands);
        // Rust format! always uses '.' — but verify explicitly
        assert!(filter.contains("3.5"));
        assert!(!filter.contains("3,5"));
    }

    #[test]
    fn test_eq_filter_string_sanitizes_nan() {
        let mut bands = [0.0f32; 10];
        bands[0] = f32::NAN;
        let filter = build_eq_filter_string(&bands);
        // NaN should be sanitized to 0.0
        assert!(filter.contains("equalizer=f=31:width_type=o:w=1:g=0.0"));
    }

    #[test]
    fn test_eq_filter_string_sanitizes_inf() {
        let mut bands = [0.0f32; 10];
        bands[0] = f32::INFINITY;
        bands[1] = f32::NEG_INFINITY;
        let filter = build_eq_filter_string(&bands);
        assert!(filter.contains("equalizer=f=31:width_type=o:w=1:g=0.0"));
        assert!(filter.contains("equalizer=f=62:width_type=o:w=1:g=0.0"));
    }

    #[test]
    fn test_eq_frequencies_count() {
        assert_eq!(EQ_FREQUENCIES.len(), 10);
        assert_eq!(EQ_FREQUENCIES[0], 31);
        assert_eq!(EQ_FREQUENCIES[9], 16000);
    }

    // --- Filename sanitization tests ---

    #[test]
    fn test_sanitize_filename_keeps_safe_chars() {
        assert_eq!(sanitize_filename("abc123_test-file"), "abc123_test-file");
    }

    #[test]
    fn test_sanitize_filename_strips_unsafe_chars() {
        assert_eq!(sanitize_filename("track/with:bad*chars"), "trackwithbadchars");
        assert_eq!(sanitize_filename("../../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_filename("file name.flac"), "filenameflac");
    }

    #[test]
    fn test_sanitize_filename_empty() {
        assert_eq!(sanitize_filename(""), "");
        assert_eq!(sanitize_filename("***"), "");
    }

    #[test]
    fn test_allowed_extension() {
        assert!(is_allowed_extension("flac"));
        assert!(is_allowed_extension("FLAC"));
        assert!(is_allowed_extension("mp3"));
        assert!(is_allowed_extension("aac"));
        assert!(is_allowed_extension("wav"));
        assert!(is_allowed_extension("ogg"));
        assert!(is_allowed_extension("opus"));
        assert!(is_allowed_extension("m4a"));
        assert!(is_allowed_extension("bin"));
        assert!(!is_allowed_extension("exe"));
        assert!(!is_allowed_extension("sh"));
        assert!(!is_allowed_extension(""));
    }

    // --- DownloadCache tests ---

    #[test]
    fn test_cache_insert_and_get() {
        let mut cache = DownloadCache::new(1_000_000);
        assert!(cache.get("1").is_none());

        cache.insert("1".into(), PathBuf::from("/tmp/1.flac"), 500_000);
        assert_eq!(cache.get("1"), Some(Path::new("/tmp/1.flac")));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_size(), 500_000);
    }

    #[test]
    fn test_cache_lru_eviction() {
        let mut cache = DownloadCache::new(1000);
        cache.insert("a".into(), PathBuf::from("/a"), 400);
        cache.insert("b".into(), PathBuf::from("/b"), 400);
        cache.insert("c".into(), PathBuf::from("/c"), 400);
        // Total = 1200 > 1000

        let evicted = cache.evict_if_needed(None);
        // Should evict oldest ("a") first
        assert!(evicted.contains(&PathBuf::from("/a")));
        assert!(cache.get("a").is_none());
        assert_eq!(cache.total_size(), 800);
    }

    #[test]
    fn test_cache_touch_updates_order() {
        let mut cache = DownloadCache::new(1000);
        cache.insert("a".into(), PathBuf::from("/a"), 400);
        cache.insert("b".into(), PathBuf::from("/b"), 400);
        cache.insert("c".into(), PathBuf::from("/c"), 400);
        // Touch "a" — makes it most recently used
        cache.touch("a");
        // Total = 1200 > 1000

        let evicted = cache.evict_if_needed(None);
        // Should evict "b" (now oldest) instead of "a"
        assert!(evicted.contains(&PathBuf::from("/b")));
        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none());
    }

    #[test]
    fn test_cache_protects_current_track() {
        let mut cache = DownloadCache::new(500);
        cache.insert("playing".into(), PathBuf::from("/playing"), 400);
        cache.insert("next".into(), PathBuf::from("/next"), 400);
        // Total = 800 > 500

        let evicted = cache.evict_if_needed(Some("playing"));
        // Should evict "next", not "playing"
        assert!(evicted.contains(&PathBuf::from("/next")));
        assert!(cache.get("playing").is_some());
    }

    #[test]
    fn test_cache_clear() {
        let mut cache = DownloadCache::new(1_000_000);
        cache.insert("a".into(), PathBuf::from("/a"), 100);
        cache.insert("b".into(), PathBuf::from("/b"), 200);

        let paths = cache.clear();
        assert_eq!(paths.len(), 2);
        assert!(cache.is_empty());
        assert_eq!(cache.total_size(), 0);
    }

    #[test]
    fn test_cache_remove() {
        let mut cache = DownloadCache::new(1_000_000);
        cache.insert("a".into(), PathBuf::from("/a"), 100);
        cache.insert("b".into(), PathBuf::from("/b"), 200);

        let removed = cache.remove("a");
        assert_eq!(removed, Some(PathBuf::from("/a")));
        assert!(cache.get("a").is_none());
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_size(), 200);
    }

    #[test]
    fn test_cache_update_existing_entry() {
        let mut cache = DownloadCache::new(1_000_000);
        cache.insert("a".into(), PathBuf::from("/old"), 100);
        cache.insert("a".into(), PathBuf::from("/new"), 200);

        assert_eq!(cache.get("a"), Some(Path::new("/new")));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.total_size(), 200);
    }

    // --- Queue operation tests ---

    #[test]
    fn test_load_queue() {
        let (player, mpv) = make_player();
        let tracks = vec![make_test_track("1"), make_test_track("2"), make_test_track("3")];

        player.load_queue(tracks.clone(), 0);

        let state = player.state();
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert_eq!(state.queue.len(), 3);
        assert_eq!(state.queue_index, 0);
        assert_eq!(state.current_track.as_ref().unwrap().rating_key, "1");

        // Verify mpv calls: 3 load_file + set_pause(false)
        let calls = mpv.calls();
        let load_files: Vec<_> = calls
            .iter()
            .filter(|c| matches!(c, MockCall::LoadFile { .. }))
            .collect();
        assert_eq!(load_files.len(), 3);

        // First should be Replace, rest Append
        assert!(matches!(&calls[0], MockCall::LoadFile { mode: LoadMode::Replace, .. }));
        assert!(matches!(&calls[1], MockCall::LoadFile { mode: LoadMode::Append, .. }));
        assert!(matches!(&calls[2], MockCall::LoadFile { mode: LoadMode::Append, .. }));
    }

    #[test]
    fn test_load_queue_at_index() {
        let (player, mpv) = make_player();
        let tracks = vec![make_test_track("1"), make_test_track("2"), make_test_track("3")];

        player.load_queue(tracks, 2);

        let state = player.state();
        assert_eq!(state.queue_index, 2);
        assert_eq!(state.current_track.as_ref().unwrap().rating_key, "3");

        // Should call playlist_play_index(2)
        let calls = mpv.calls();
        assert!(calls
            .iter()
            .any(|c| matches!(c, MockCall::PlaylistPlayIndex(2))));
    }

    #[test]
    fn test_load_queue_empty_is_noop() {
        let (player, mpv) = make_player();
        player.load_queue(vec![], 0);
        assert_eq!(player.state().status, PlaybackStatus::Stopped);
        assert_eq!(mpv.call_count(), 0);
    }

    #[test]
    fn test_load_queue_out_of_bounds_is_noop() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 5);
        assert_eq!(player.state().status, PlaybackStatus::Stopped);
        assert_eq!(mpv.call_count(), 0);
    }

    #[test]
    fn test_append_to_queue() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        let initial_calls = mpv.call_count();

        player.append_to_queue(vec![make_test_track("2"), make_test_track("3")]);

        let state = player.state();
        assert_eq!(state.queue.len(), 3);
        assert_eq!(state.queue_index, 0); // Still on first track

        // Should have 2 Append load_file calls
        let new_calls = &mpv.calls()[initial_calls..];
        let appends: Vec<_> = new_calls
            .iter()
            .filter(|c| matches!(c, MockCall::LoadFile { mode: LoadMode::Append, .. }))
            .collect();
        assert_eq!(appends.len(), 2);
    }

    #[test]
    fn test_append_to_queue_auto_start() {
        let (player, _mpv) = make_player();
        // Player starts stopped with empty queue
        player.append_to_queue(vec![make_test_track("1"), make_test_track("2")]);

        let state = player.state();
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert_eq!(state.queue.len(), 2);
        assert_eq!(state.queue_index, 0);
    }

    #[test]
    fn test_insert_next() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("3")],
            0,
        );
        let initial_calls = mpv.call_count();

        player.insert_next(vec![make_test_track("2")]);

        let state = player.state();
        assert_eq!(state.queue.len(), 3);
        // Inserted after current (index 0), so at index 1
        assert_eq!(state.queue[1].rating_key, "2");
        assert_eq!(state.queue[2].rating_key, "3");

        // Should use load_file_at with index 1
        let new_calls = &mpv.calls()[initial_calls..];
        assert!(new_calls
            .iter()
            .any(|c| matches!(c, MockCall::LoadFileAt { index: 1, .. })));
    }

    #[test]
    fn test_insert_next_when_stopped_becomes_load() {
        let (player, _mpv) = make_player();
        player.insert_next(vec![make_test_track("1")]);

        let state = player.state();
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert_eq!(state.queue.len(), 1);
    }

    #[test]
    fn test_remove_from_queue() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
            0,
        );
        let initial_calls = mpv.call_count();

        player.remove_from_queue(2); // Remove last track

        let state = player.state();
        assert_eq!(state.queue.len(), 2);
        assert_eq!(state.queue_index, 0); // Unchanged

        let new_calls = &mpv.calls()[initial_calls..];
        assert!(new_calls
            .iter()
            .any(|c| matches!(c, MockCall::PlaylistRemove(2))));
    }

    #[test]
    fn test_remove_current_track_is_noop() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2")],
            0,
        );
        let initial_calls = mpv.call_count();

        player.remove_from_queue(0); // Current track

        assert_eq!(player.state().queue.len(), 2); // Unchanged
        assert_eq!(mpv.call_count(), initial_calls); // No mpv calls
    }

    #[test]
    fn test_remove_before_current_adjusts_index() {
        let (player, _mpv) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
            1, // Playing track "2"
        );

        player.remove_from_queue(0); // Remove track before current

        let state = player.state();
        assert_eq!(state.queue_index, 0); // Adjusted from 1 to 0
        assert_eq!(state.queue.len(), 2);
    }

    #[test]
    fn test_jump_to_index() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
            0,
        );
        let initial_calls = mpv.call_count();

        player.jump_to_index(2);

        let state = player.state();
        assert_eq!(state.queue_index, 2);
        assert_eq!(state.current_track.as_ref().unwrap().rating_key, "3");

        let new_calls = &mpv.calls()[initial_calls..];
        assert!(new_calls
            .iter()
            .any(|c| matches!(c, MockCall::PlaylistPlayIndex(2))));
    }

    #[test]
    fn test_next() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2")],
            0,
        );
        let initial_calls = mpv.call_count();

        player.next();

        let state = player.state();
        assert_eq!(state.queue_index, 1);
        assert_eq!(state.current_track.as_ref().unwrap().rating_key, "2");

        let new_calls = &mpv.calls()[initial_calls..];
        assert!(new_calls
            .iter()
            .any(|c| matches!(c, MockCall::PlaylistPlayIndex(1))));
    }

    #[test]
    fn test_next_at_end_stops() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        let initial_calls = mpv.call_count();

        player.next();

        let state = player.state();
        assert_eq!(state.status, PlaybackStatus::Stopped);
        assert!(state.current_track.is_none());

        let new_calls = &mpv.calls()[initial_calls..];
        assert!(new_calls.iter().any(|c| matches!(c, MockCall::Stop)));
    }

    #[test]
    fn test_previous_restarts_if_past_threshold() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2")],
            1,
        );
        player.handle_position_change(5.0); // > 3s
        let initial_calls = mpv.call_count();

        player.previous();

        let state = player.state();
        assert_eq!(state.queue_index, 1); // Didn't change track
        assert_eq!(player.position(), 0.0);

        let new_calls = &mpv.calls()[initial_calls..];
        assert!(new_calls
            .iter()
            .any(|c| matches!(c, MockCall::Seek(pos) if *pos == 0.0)));
    }

    #[test]
    fn test_previous_goes_back_if_within_threshold() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2")],
            1,
        );
        player.handle_position_change(1.0); // < 3s
        let initial_calls = mpv.call_count();

        player.previous();

        let state = player.state();
        assert_eq!(state.queue_index, 0);
        assert_eq!(state.current_track.as_ref().unwrap().rating_key, "1");

        let new_calls = &mpv.calls()[initial_calls..];
        assert!(new_calls
            .iter()
            .any(|c| matches!(c, MockCall::PlaylistPlayIndex(0))));
    }

    #[test]
    fn test_previous_at_start_seeks_to_zero() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_position_change(1.0); // < 3s, but at index 0
        let initial_calls = mpv.call_count();

        player.previous();

        assert_eq!(player.state().queue_index, 0);
        let new_calls = &mpv.calls()[initial_calls..];
        assert!(new_calls
            .iter()
            .any(|c| matches!(c, MockCall::Seek(pos) if *pos == 0.0)));
    }

    #[test]
    fn test_toggle_play_pause() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        assert_eq!(player.state().status, PlaybackStatus::Playing);

        player.toggle_play_pause();
        assert_eq!(player.state().status, PlaybackStatus::Paused);

        player.toggle_play_pause();
        assert_eq!(player.state().status, PlaybackStatus::Playing);

        let calls = mpv.calls();
        let pause_calls: Vec<_> = calls
            .iter()
            .filter(|c| matches!(c, MockCall::SetPause(_)))
            .collect();
        assert!(pause_calls.len() >= 2);
    }

    #[test]
    fn test_toggle_when_stopped_is_noop() {
        let (player, mpv) = make_player();
        let initial_calls = mpv.call_count();
        player.toggle_play_pause();
        assert_eq!(mpv.call_count(), initial_calls);
    }

    #[test]
    fn test_seek() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_duration_change(180.0);
        let initial_calls = mpv.call_count();

        player.seek(60.0);

        assert!((player.position() - 60.0).abs() < 0.1);
        let new_calls = &mpv.calls()[initial_calls..];
        assert!(new_calls
            .iter()
            .any(|c| matches!(c, MockCall::Seek(pos) if (*pos - 60.0).abs() < 0.1)));
    }

    #[test]
    fn test_seek_clamps_to_bounds() {
        let (player, _mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_duration_change(180.0);

        player.seek(-10.0);
        assert!(player.position() >= 0.0);

        player.seek(999.0);
        // Should clamp to duration - 0.5
        assert!(player.position() <= 179.5);
    }

    #[test]
    fn test_set_volume() {
        let (player, mpv) = make_player();
        player.set_volume(75.0);

        assert!((player.volume() - 75.0).abs() < 0.1);
        let calls = mpv.calls();
        assert!(calls
            .iter()
            .any(|c| matches!(c, MockCall::SetVolume(v) if (*v - 75.0).abs() < 0.1)));
    }

    #[test]
    fn test_set_volume_clamps() {
        let (player, _mpv) = make_player();
        player.set_volume(150.0);
        assert!((player.volume() - 100.0).abs() < 0.1);

        player.set_volume(-10.0);
        assert!(player.volume() >= 0.0);
    }

    #[test]
    fn test_stop() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2")],
            0,
        );
        let initial_calls = mpv.call_count();

        player.stop();

        let state = player.state();
        assert_eq!(state.status, PlaybackStatus::Stopped);
        assert!(state.current_track.is_none());
        assert!(state.queue.is_empty());
        assert_eq!(state.queue_index, 0);

        let new_calls = &mpv.calls()[initial_calls..];
        assert!(new_calls.iter().any(|c| matches!(c, MockCall::Stop)));
    }

    // --- Equalizer integration tests ---

    #[test]
    fn test_apply_equalizer_enabled() {
        let (player, mpv) = make_player();
        let bands = [3.0, -1.0, 0.0, 2.0, -2.0, 1.0, 0.5, -0.5, 4.0, -4.0];
        player.apply_equalizer(true, &bands);

        let calls = mpv.calls();
        let filter_call = calls
            .iter()
            .find(|c| matches!(c, MockCall::SetAudioFilters(_)));
        assert!(filter_call.is_some());
        if let MockCall::SetAudioFilters(s) = filter_call.unwrap() {
            assert!(s.starts_with("lavfi=["));
        }
    }

    #[test]
    fn test_apply_equalizer_disabled() {
        let (player, mpv) = make_player();
        let bands = [0.0; 10];
        player.apply_equalizer(false, &bands);

        let calls = mpv.calls();
        assert!(calls
            .iter()
            .any(|c| matches!(c, MockCall::SetAudioFilters(s) if s == "no")));
    }

    // --- Callback handler tests ---

    #[test]
    fn test_handle_position_change() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);

        player.handle_position_change(42.5);
        assert!((player.position() - 42.5).abs() < 0.01);
    }

    #[test]
    fn test_handle_duration_change() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);

        player.handle_duration_change(200.0);
        assert!((player.duration() - 200.0).abs() < 0.01);
    }

    #[test]
    fn test_handle_playlist_pos_change() {
        let (player, _) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
            0,
        );

        player.handle_playlist_pos_change(2);

        let state = player.state();
        assert_eq!(state.queue_index, 2);
        assert_eq!(state.current_track.as_ref().unwrap().rating_key, "3");
        assert_eq!(player.position(), 0.0);
    }

    #[test]
    fn test_handle_playlist_pos_change_negative_is_ignored() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);

        player.handle_playlist_pos_change(-1);
        assert_eq!(player.state().queue_index, 0);
    }

    #[test]
    fn test_handle_pause_change() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        assert_eq!(player.state().status, PlaybackStatus::Playing);

        player.handle_pause_change(true);
        assert_eq!(player.state().status, PlaybackStatus::Paused);

        player.handle_pause_change(false);
        assert_eq!(player.state().status, PlaybackStatus::Playing);
    }

    #[test]
    fn test_handle_idle_active() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);

        player.handle_idle_active();

        let state = player.state();
        assert_eq!(state.status, PlaybackStatus::Stopped);
        assert!(state.current_track.is_none());
    }

    #[test]
    fn test_handle_file_loaded_clears_loading() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        {
            let mut inner = player.inner.lock();
            inner.is_loading = true;
            inner.is_buffering = true;
        }

        player.handle_file_loaded();

        let snapshot = player.snapshot();
        assert!(!snapshot.is_loading);
        assert!(!snapshot.is_buffering);
    }

    #[test]
    fn test_handle_file_ended_error_skips() {
        let (player, _) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2")],
            0,
        );

        player.handle_file_ended(FileEndReason::Error("test".into()));

        // Should have advanced to next track
        let state = player.state();
        assert_eq!(state.queue_index, 1);
    }

    #[test]
    fn test_handle_cache_state_change() {
        let (player, _) = make_player();
        player.handle_cache_state_change(50);
        let snapshot = player.snapshot();
        assert!((snapshot.buffered_fraction - 0.5).abs() < 0.01);
    }

    // --- URL resolution tests ---

    #[test]
    fn test_load_queue_resolves_direct_play_urls() {
        let (player, mpv) = make_player();
        let track = make_test_track("123");
        player.load_queue(vec![track], 0);

        let calls = mpv.calls();
        let load = calls
            .iter()
            .find(|c| matches!(c, MockCall::LoadFile { .. }));
        assert!(load.is_some());
        if let MockCall::LoadFile { url, .. } = load.unwrap() {
            assert!(url.contains("test-token"));
            assert!(url.contains("/library/parts/123/file.flac"));
        }
    }

    #[test]
    fn test_cached_track_uses_file_url() {
        let (player, mpv) = make_player();

        // Pre-populate cache
        player.with_cache(|cache| {
            cache.insert(
                "123".into(),
                PathBuf::from("/tmp/cache/123.flac"),
                1000,
            );
        });

        let track = make_test_track("123");
        player.load_queue(vec![track], 0);

        let calls = mpv.calls();
        let load = calls
            .iter()
            .find(|c| matches!(c, MockCall::LoadFile { .. }));
        if let Some(MockCall::LoadFile { url, .. }) = load {
            assert!(url.starts_with("file://"));
            assert!(url.contains("/tmp/cache/123.flac"));
        }
    }

    #[test]
    fn test_snapshot_reflects_state() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_position_change(30.0);
        player.handle_duration_change(180.0);
        player.set_volume(80.0);

        let snapshot = player.snapshot();
        assert_eq!(snapshot.state.status, PlaybackStatus::Playing);
        assert!((snapshot.position - 30.0).abs() < 0.1);
        assert!((snapshot.duration - 180.0).abs() < 0.1);
        assert!((snapshot.volume - 80.0).abs() < 0.1);
    }

    #[test]
    fn test_load_queue_generates_new_session_id() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        let session1 = player.play_session_id();

        player.load_queue(vec![make_test_track("2")], 0);
        let session2 = player.play_session_id();

        assert_ne!(session1, session2);
    }
}
