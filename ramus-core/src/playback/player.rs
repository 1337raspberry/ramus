//! AudioPlayer: queue management, mpv integration, equalizer, download
//! cache. Owns the mpv handle (via `MpvPlayer` trait) and manages the
//! playback queue, track URL resolution, LRU download cache, and 10-band
//! parametric equalizer filter strings.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use url::Url;

use crate::models::{PlaybackConfig, PlaybackStatus, PlayerState, Track};
use crate::playback::download_cache::DownloadCache;
use crate::playback::mpv::{FileEndReason, LoadMode, MpvPlayer};
use crate::playback::transcode;

/// 10-band EQ center frequencies in Hz.
pub const EQ_FREQUENCIES: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

/// Allowed file extensions for cached audio files.
const ALLOWED_EXTENSIONS: &[&str] = &[
    "flac", "alac", "m4a", "mp3", "aac", "wav", "aiff", "ogg", "opus", "mp2", "bin",
];

/// Threshold in seconds: if position > this, `previous()` restarts instead of going back.
const PREVIOUS_RESTART_THRESHOLD: f64 = 3.0;

/// Build an mpv `af` lavfi equalizer filter string from 10 gain values.
///
/// Rust's `format!` always uses `.` for decimals. NaN and Inf values are
/// sanitized to 0.0.
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

/// Build the mpv `af` chain string for the current EQ state.
///
/// When EQ is enabled, returns the 10-band lavfi equalizer chain. When
/// disabled, returns an empty string — `set_audio_filters("")` interprets
/// this as "no filters", clearing anything previously set.
pub fn build_af_string(eq_enabled: bool, bands: &[f32; 10]) -> String {
    if eq_enabled {
        build_eq_filter_string(bands)
    } else {
        String::new()
    }
}

/// Sanitize a string for use as a filename. Only `[a-zA-Z0-9_-]` are kept.
pub fn sanitize_filename(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

/// Whether a file extension is in the allowed set for audio caching.
pub fn is_allowed_extension(ext: &str) -> bool {
    ALLOWED_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}

/// Observable state snapshot for the frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioPlayerState {
    pub state: PlayerState,
    pub position: f64,
    pub duration: f64,
    pub is_loading: bool,
    pub waveform_levels: Option<Vec<f32>>,
    pub volume: f64,
}

struct PlayerInner {
    state: PlayerState,
    position: f64,
    duration: f64,
    is_loading: bool,
    volume: f64,
    config: PlaybackConfig,
    server_url: Option<Url>,
    token: Option<String>,
    client_identifier: String,
    is_remote: bool,
    play_session_id: String,
    cache: DownloadCache,
}

/// Core audio player managing queue state, mpv commands, and track resolution.
pub struct AudioPlayer {
    mpv: Arc<dyn MpvPlayer>,
    inner: Mutex<PlayerInner>,
    /// Permanent downloads. Checked before the LRU prefetch cache when
    /// resolving a track's URL — if a track is here, playback always uses
    /// the local file, online or offline. Populated at startup from the
    /// `downloads` DB table and on every successful user download.
    persistent_cache: RwLock<HashMap<String, PathBuf>>,
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
                volume: 100.0,
                config: PlaybackConfig::default(),
                server_url: None,
                token: None,
                client_identifier: String::new(),
                is_remote: false,
                play_session_id: uuid::Uuid::new_v4().to_string(),
                cache: DownloadCache::new(PlaybackConfig::DEFAULT_CACHE_LIMIT_BYTES as u64),
            }),
            persistent_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Register a downloaded file as permanently cached. Takes priority
    /// over the LRU prefetch cache in `resolve_url`.
    pub fn register_persistent_download(&self, rating_key: String, path: PathBuf) {
        self.persistent_cache.write().insert(rating_key, path);
    }

    /// Unregister a downloaded file (e.g. user removed it from the downloads panel).
    pub fn unregister_persistent_download(&self, rating_key: &str) {
        self.persistent_cache.write().remove(rating_key);
    }

    /// Replace the entire persistent cache. Called once at app startup
    /// after loading the `downloads` table.
    pub fn rehydrate_persistent_cache(&self, entries: HashMap<String, PathBuf>) {
        *self.persistent_cache.write() = entries;
    }

    /// Whether a rating key has a permanent download on disk.
    pub fn has_persistent_download(&self, rating_key: &str) -> bool {
        self.persistent_cache.read().contains_key(rating_key)
    }

    /// Snapshot of all persistent download paths. Used by the downloads panel.
    pub fn persistent_download_paths(&self) -> HashMap<String, PathBuf> {
        self.persistent_cache.read().clone()
    }

    /// Set server connection details.
    pub fn configure(&self, server_url: Url, token: String, client_identifier: String) {
        // DEBUG-ONLY: leaks the user's plex.direct subdomain hash (derived
        // from server IP + a plex.tv-issued UUID). Do NOT promote to
        // info/warn without scrubbing the host.
        log::debug!("player.configure: server_url={server_url}");
        let mut inner = self.inner.lock();
        inner.server_url = Some(server_url);
        inner.token = Some(token);
        inner.client_identifier = client_identifier;
    }

    /// Update server connection (e.g., after failover or reconnection).
    pub fn update_server_connection(&self, server_url: Url, token: String, is_remote: bool) {
        // DEBUG-ONLY: see note on configure() above.
        log::debug!(
            "player.update_server_connection: server_url={server_url} is_remote={is_remote}"
        );
        let mut inner = self.inner.lock();
        inner.server_url = Some(server_url);
        inner.token = Some(token);
        inner.is_remote = is_remote;
    }

    /// Update only the remote flag.
    pub fn set_remote(&self, is_remote: bool) {
        self.inner.lock().is_remote = is_remote;
    }

    /// Whether the current connection is remote. Feeds into
    /// `should_transcode()` under `TranscodeLosslessRemote`.
    pub fn is_remote(&self) -> bool {
        self.inner.lock().is_remote
    }

    /// Update playback configuration.
    pub fn update_config(&self, config: PlaybackConfig) {
        let mut inner = self.inner.lock();
        inner.cache.limit_bytes = config.audio_cache_limit_bytes as u64;
        inner.config = config;
    }

    /// Replace the queue and start playback at the given index.
    ///
    /// Uses `loadfile "replace"` for the first track and `"append"` for
    /// the rest. Does NOT call `mpv.stop()` first — `replace` handles
    /// that implicitly.
    pub fn load_queue(&self, tracks: Vec<Track>, start_at: usize) {
        if tracks.is_empty() || start_at >= tracks.len() {
            return;
        }

        // Snapshot per-track URLs under the lock, then release before
        // touching mpv (FFI calls may block briefly).
        let loads: Vec<Option<String>> = {
            let persistent = self.persistent_cache.read();
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
                .map(|t| resolve_url(t, &inner, &persistent))
                .collect()
        };

        for (i, load) in loads.iter().enumerate() {
            if let Some(url) = load {
                let mode = if i == 0 {
                    LoadMode::Replace
                } else {
                    LoadMode::Append
                };
                log::debug!("load_queue[{i}]: mode={mode:?} url={url}");
                self.mpv.load_file(url, mode, None);
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

        let (was_stopped, loads) = {
            let persistent = self.persistent_cache.read();
            let mut inner = self.inner.lock();
            let was_stopped =
                inner.state.queue.is_empty() || inner.state.status == PlaybackStatus::Stopped;
            inner.state.queue.extend(tracks.iter().cloned());

            if was_stopped {
                (true, Vec::new())
            } else {
                let loads: Vec<Option<String>> = tracks
                    .iter()
                    .map(|t| resolve_url(t, &inner, &persistent))
                    .collect();
                (false, loads)
            }
        };

        if was_stopped {
            let queue = self.inner.lock().state.queue.clone();
            self.load_queue(queue, 0);
        } else {
            for url in loads.into_iter().flatten() {
                self.mpv.load_file(&url, LoadMode::Append, None);
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

        let (insert_base, loads) = {
            let persistent = self.persistent_cache.read();
            let mut inner = self.inner.lock();
            let insert_base = inner.state.queue_index + 1;

            for (offset, track) in tracks.iter().enumerate() {
                inner
                    .state
                    .queue
                    .insert(insert_base + offset, track.clone());
            }

            let loads: Vec<Option<String>> = tracks
                .iter()
                .map(|t| resolve_url(t, &inner, &persistent))
                .collect();
            (insert_base, loads)
        };

        for (offset, load) in loads.iter().enumerate() {
            if let Some(url) = load {
                self.mpv
                    .load_file_at(url, (insert_base + offset) as i64, None);
            }
        }
    }

    /// Remove a track from the queue by index. Cannot remove the currently
    /// playing track. Adjusts queue index if needed.
    pub fn remove_from_queue(&self, index: usize) {
        let mut inner = self.inner.lock();

        if index == inner.state.queue_index {
            return;
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

    /// Unconditionally pause playback. Safe to call when already paused.
    pub fn pause(&self) {
        let mut inner = self.inner.lock();
        if inner.state.status == PlaybackStatus::Playing {
            inner.state.status = PlaybackStatus::Paused;
            drop(inner);
            self.mpv.set_pause(true);
        }
    }

    /// Unconditionally resume playback. Safe to call when already playing.
    pub fn resume(&self) {
        let mut inner = self.inner.lock();
        if inner.state.status == PlaybackStatus::Paused {
            inner.state.status = PlaybackStatus::Playing;
            drop(inner);
            self.mpv.set_pause(false);
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

    /// Apply or clear the 10-band equalizer. When `enabled` is false the
    /// `af` chain is cleared entirely.
    pub fn apply_equalizer(&self, enabled: bool, bands: &[f32; 10]) {
        let filter = build_af_string(enabled, bands);
        self.mpv.set_audio_filters(&filter);
    }

    /// Snapshot the full player state for the frontend.
    pub fn snapshot(&self) -> AudioPlayerState {
        let inner = self.inner.lock();
        AudioPlayerState {
            state: inner.state.clone(),
            position: inner.position,
            duration: inner.duration,
            is_loading: inner.is_loading,
            waveform_levels: None,
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

    /// Handle mpv position change (called by event loop, ~30fps).
    pub fn handle_position_change(&self, pos: f64) {
        self.inner.lock().position = pos;
    }

    /// Handle mpv duration change.
    pub fn handle_duration_change(&self, dur: f64) {
        self.inner.lock().duration = dur;
    }

    /// Handle mpv playlist-pos change (track advance).
    ///
    /// Resets position but NOT duration. For manual skips, the caller
    /// already resets duration before calling `playlist_play_index`. For
    /// gapless auto-advance, mpv's `prefetch-playlist` pre-demuxes the
    /// next file and may have already delivered the correct duration via
    /// `on_duration_change`. Resetting it to 0 here would cause every
    /// subsequent `on_position_change` tick to emit `duration=0` to the
    /// frontend (since `observe_property` won't re-fire for a value that
    /// hasn't changed from mpv's perspective), breaking the seek bar.
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

    /// Handle mpv file-loaded event.
    pub fn handle_file_loaded(&self) {
        self.inner.lock().is_loading = false;
    }

    /// Handle mpv file-ended event.
    pub fn handle_file_ended(&self, reason: FileEndReason) {
        match reason {
            FileEndReason::Eof => {
                // Natural end — mpv auto-advances via gapless playback;
                // if last track, idle-active will fire.
            }
            FileEndReason::Error(_) => {
                self.next();
            }
            _ => {}
        }
    }

    /// Handle mpv idle-active (queue completed).
    pub fn handle_idle_active(&self) {
        let mut inner = self.inner.lock();
        inner.state.status = PlaybackStatus::Stopped;
        inner.state.current_track = None;
        inner.position = 0.0;
    }

    /// Access the download cache under the player lock.
    pub fn with_cache<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut DownloadCache) -> R,
    {
        let mut inner = self.inner.lock();
        f(&mut inner.cache)
    }

    /// Returns `(rating_key, direct_play_url)` for the first uncached,
    /// non-transcode track within `lookahead_depth` of the current queue
    /// position. Walks forward past already-cached entries. Returns
    /// `None` when every slot in the window is cached, transcoded, or
    /// out of bounds.
    ///
    /// Called fresh on every iteration of the prefetch worker's serial
    /// loop, so it auto-reflects queue advancement.
    pub fn next_uncached_target_in_lookahead(&self, include_current: bool) -> Option<(String, String)> {
        let persistent = self.persistent_cache.read();
        let inner = self.inner.lock();
        let depth = inner.config.lookahead_depth as usize;
        let pos = inner.state.queue_index;
        let server_url = inner.server_url.as_ref()?;
        let token = inner.token.as_ref()?;

        let start_offset = if include_current { 0 } else { 1 };
        for offset in start_offset..=depth {
            let idx = pos + offset;
            let track = inner.state.queue.get(idx)?;

            if persistent.contains_key(&track.rating_key)
                || inner.cache.get(&track.rating_key).is_some()
            {
                continue;
            }
            if transcode::should_transcode(
                track.codec.as_deref(),
                inner.config.playback_mode,
                inner.is_remote,
            ) {
                continue;
            }

            let Some(part_key) = track.part_key.as_ref() else {
                continue;
            };
            let Some(url) = transcode::build_direct_play_url(server_url, part_key, token) else {
                continue;
            };
            // DEBUG-ONLY, SENSITIVE: the URL contains a live `X-Plex-Token`
            // query parameter. Do NOT promote above `debug!`, and scrub
            // the token before copying into a bug report.
            log::debug!(
                "next_uncached_target: idx={idx} rk={} server={} part_key={} -> {}",
                track.rating_key,
                server_url,
                part_key,
                url,
            );
            return Some((track.rating_key.clone(), url.to_string()));
        }
        None
    }

    /// Rating key of the currently playing track (for cache eviction protection).
    pub fn current_track_id(&self) -> Option<String> {
        self.inner
            .lock()
            .state
            .current_track
            .as_ref()
            .map(|t| t.rating_key.clone())
    }

    /// Returns `(rating_key, local_path)` for every track in the current
    /// playback queue's lookahead window that is already available on
    /// disk — either in the LRU prefetch cache or as a permanent download.
    /// Used by the download worker to drive spectrum analysis for
    /// already-cached tracks, which no longer trigger the prefetch
    /// success path that historically queued analysis.
    pub fn cached_paths_in_lookahead(
        &self,
        include_current: bool,
    ) -> Vec<(String, PathBuf)> {
        let persistent = self.persistent_cache.read();
        let inner = self.inner.lock();
        let depth = inner.config.lookahead_depth as usize;
        let pos = inner.state.queue_index;
        let start_offset = if include_current { 0 } else { 1 };
        let mut out = Vec::new();
        for offset in start_offset..=depth {
            let idx = pos + offset;
            let Some(track) = inner.state.queue.get(idx) else {
                break;
            };
            if let Some(path) = persistent.get(&track.rating_key) {
                out.push((track.rating_key.clone(), path.clone()));
                continue;
            }
            if let Some(path) = inner.cache.get(&track.rating_key) {
                out.push((track.rating_key.clone(), path.to_path_buf()));
            }
        }
        out
    }

    /// Swap a cached track's mpv playlist entry to `file://<path>` so mpv
    /// reads from the local cache file instead of re-downloading.
    ///
    /// Called by the prefetch worker after every successful download.
    /// No-op if the track isn't in the current queue or is the currently
    /// playing entry (mpv refuses to playlist-remove the active index).
    pub fn swap_playlist_entry_to_cached(&self, track_id: &str) {
        let (idx, file_url) = {
            let persistent = self.persistent_cache.read();
            let inner = self.inner.lock();
            let Some(idx) = inner
                .state
                .queue
                .iter()
                .position(|t| t.rating_key == track_id)
            else {
                return;
            };
            if idx == inner.state.queue_index {
                return;
            }
            let path = persistent
                .get(track_id)
                .cloned()
                .or_else(|| inner.cache.get(track_id).map(|p| p.to_path_buf()));
            let Some(path) = path else {
                return;
            };
            (idx, format!("file://{}", path.display()))
        };
        self.mpv.playlist_remove(idx as i64);
        self.mpv.load_file_at(&file_url, idx as i64, None);
    }

    /// Whether the given track would get transcoded under the current settings.
    ///
    /// Used by the focus-mode visualiser's placeholder logic: transcoded
    /// tracks can't be analysed (symphonia can't decode HLS manifests),
    /// so `get_spectrum` surfaces `Unavailable { reason: "transcoding" }`
    /// immediately instead of stranding the UI on "Analysing…".
    ///
    /// Returns `false` for rating keys not in the current queue; the
    /// caller falls through to the normal "analysis pending" path, which
    /// self-corrects once the track joins the queue.
    pub fn would_transcode(&self, rating_key: &str) -> bool {
        let inner = self.inner.lock();
        let Some(track) = inner.state.queue.iter().find(|t| t.rating_key == rating_key) else {
            return false;
        };
        transcode::should_transcode(
            track.codec.as_deref(),
            inner.config.playback_mode,
            inner.is_remote,
        )
    }
}

fn resolve_url(
    track: &Track,
    inner: &PlayerInner,
    persistent: &HashMap<String, PathBuf>,
) -> Option<String> {
    if let Some(path) = persistent.get(&track.rating_key) {
        return Some(format!("file://{}", path.display()));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::mpv::MpvPlayer;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Debug, Clone)]
    #[allow(dead_code)] // Fields read via Debug/pattern matching in assertions
    enum MockCall {
        LoadFile {
            url: String,
            mode: LoadMode,
            options: Option<String>,
        },
        LoadFileAt {
            url: String,
            index: i64,
            options: Option<String>,
        },
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
                volume: Mutex::new(100.0),
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
        fn load_file(&self, url: &str, mode: LoadMode, options: Option<&str>) {
            self.calls.lock().push(MockCall::LoadFile {
                url: url.to_string(),
                mode,
                options: options.map(|s| s.to_string()),
            });
        }
        fn load_file_at(&self, url: &str, index: i64, options: Option<&str>) {
            self.calls.lock().push(MockCall::LoadFileAt {
                url: url.to_string(),
                index,
                options: options.map(|s| s.to_string()),
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
            file_size_bytes: None,
            rating_count: None,
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

    #[test]
    fn test_eq_filter_string_all_zeros() {
        let bands = [0.0f32; 10];
        let filter = build_eq_filter_string(&bands);
        assert!(filter.starts_with("lavfi=["));
        assert!(filter.ends_with(']'));
        assert!(filter.contains("equalizer=f=31:width_type=o:w=1:g=0.0"));
        assert!(filter.contains("equalizer=f=16000:width_type=o:w=1:g=0.0"));
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
        assert!(filter.contains("3.5"));
        assert!(!filter.contains("3,5"));
    }

    #[test]
    fn test_eq_filter_string_sanitizes_nan() {
        let mut bands = [0.0f32; 10];
        bands[0] = f32::NAN;
        let filter = build_eq_filter_string(&bands);
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

        let calls = mpv.calls();
        let load_files: Vec<_> = calls
            .iter()
            .filter(|c| matches!(c, MockCall::LoadFile { .. }))
            .collect();
        assert_eq!(load_files.len(), 3);

        assert!(matches!(load_files[0], MockCall::LoadFile { mode: LoadMode::Replace, .. }));
        assert!(matches!(load_files[1], MockCall::LoadFile { mode: LoadMode::Append, .. }));
        assert!(matches!(load_files[2], MockCall::LoadFile { mode: LoadMode::Append, .. }));
    }

    #[test]
    fn test_load_queue_at_index() {
        let (player, mpv) = make_player();
        let tracks = vec![make_test_track("1"), make_test_track("2"), make_test_track("3")];

        player.load_queue(tracks, 2);

        let state = player.state();
        assert_eq!(state.queue_index, 2);
        assert_eq!(state.current_track.as_ref().unwrap().rating_key, "3");

        let calls = mpv.calls();
        assert!(calls
            .iter()
            .any(|c| matches!(c, MockCall::PlaylistPlayIndex(2))));
    }

    #[test]
    fn test_load_queue_empty_is_noop() {
        let (player, mpv) = make_player();
        let initial_count = mpv.call_count();
        player.load_queue(vec![], 0);
        assert_eq!(player.state().status, PlaybackStatus::Stopped);
        assert_eq!(mpv.call_count(), initial_count);
    }

    #[test]
    fn test_load_queue_out_of_bounds_is_noop() {
        let (player, mpv) = make_player();
        let initial_count = mpv.call_count();
        player.load_queue(vec![make_test_track("1")], 5);
        assert_eq!(player.state().status, PlaybackStatus::Stopped);
        assert_eq!(mpv.call_count(), initial_count);
    }

    #[test]
    fn test_append_to_queue() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        let initial_calls = mpv.call_count();

        player.append_to_queue(vec![make_test_track("2"), make_test_track("3")]);

        let state = player.state();
        assert_eq!(state.queue.len(), 3);
        assert_eq!(state.queue_index, 0);

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
        assert_eq!(state.queue[1].rating_key, "2");
        assert_eq!(state.queue[2].rating_key, "3");

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

        player.remove_from_queue(2);

        let state = player.state();
        assert_eq!(state.queue.len(), 2);
        assert_eq!(state.queue_index, 0);

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

        player.remove_from_queue(0);

        assert_eq!(player.state().queue.len(), 2);
        assert_eq!(mpv.call_count(), initial_calls);
    }

    #[test]
    fn test_remove_before_current_adjusts_index() {
        let (player, _mpv) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
            1,
        );

        player.remove_from_queue(0);

        let state = player.state();
        assert_eq!(state.queue_index, 0);
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
        player.handle_position_change(5.0);
        let initial_calls = mpv.call_count();

        player.previous();

        let state = player.state();
        assert_eq!(state.queue_index, 1);
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
        player.handle_position_change(1.0);
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
        player.handle_position_change(1.0);
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

    #[test]
    fn test_apply_equalizer_enabled() {
        let (player, mpv) = make_player();
        let bands = [3.0, -1.0, 0.0, 2.0, -2.0, 1.0, 0.5, -0.5, 4.0, -4.0];
        player.apply_equalizer(true, &bands);

        let calls = mpv.calls();
        let last_filter = calls
            .iter()
            .rev()
            .find_map(|c| match c {
                MockCall::SetAudioFilters(s) => Some(s.clone()),
                _ => None,
            })
            .expect("expected set_audio_filters to be called");
        assert!(last_filter.contains("lavfi=[equalizer="));
    }

    #[test]
    fn test_apply_equalizer_disabled() {
        let (player, mpv) = make_player();
        let bands = [0.0; 10];
        player.apply_equalizer(false, &bands);

        let calls = mpv.calls();
        let last_filter = calls
            .iter()
            .rev()
            .find_map(|c| match c {
                MockCall::SetAudioFilters(s) => Some(s.clone()),
                _ => None,
            })
            .expect("expected set_audio_filters to be called");
        assert_eq!(last_filter, "");
    }

    #[test]
    fn test_audio_player_new_does_not_touch_filters() {
        let (_player, mpv) = make_player();
        let calls = mpv.calls();
        assert!(!calls
            .iter()
            .any(|c| matches!(c, MockCall::SetAudioFilters(_))));
    }

    #[test]
    fn test_build_af_string_disabled() {
        let s = build_af_string(false, &[0.0; 10]);
        assert_eq!(s, "");
    }

    #[test]
    fn test_build_af_string_enabled() {
        let bands = [1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let s = build_af_string(true, &bands);
        assert!(s.starts_with("lavfi=[equalizer="));
        assert!(s.contains("g=1.0"));
        assert!(s.contains("g=2.0"));
        assert!(s.contains("g=3.0"));
    }

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
        }

        player.handle_file_loaded();

        let snapshot = player.snapshot();
        assert!(!snapshot.is_loading);
    }

    #[test]
    fn test_handle_file_ended_error_skips() {
        let (player, _) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2")],
            0,
        );

        player.handle_file_ended(FileEndReason::Error("test".into()));

        let state = player.state();
        assert_eq!(state.queue_index, 1);
    }

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
    fn test_persistent_download_wins_over_lru_cache() {
        let (player, mpv) = make_player();

        // LRU says /tmp/cache, persistent says /tmp/downloads — persistent wins.
        player.with_cache(|cache| {
            cache.insert(
                "123".into(),
                PathBuf::from("/tmp/cache/123.flac"),
                1000,
            );
        });
        player.register_persistent_download(
            "123".into(),
            PathBuf::from("/tmp/downloads/123.flac"),
        );

        let track = make_test_track("123");
        player.load_queue(vec![track], 0);

        let calls = mpv.calls();
        let load = calls
            .iter()
            .find(|c| matches!(c, MockCall::LoadFile { .. }));
        match load {
            Some(MockCall::LoadFile { url, .. }) => {
                assert!(url.starts_with("file://"));
                assert!(
                    url.contains("/tmp/downloads/123.flac"),
                    "persistent download should win, got {url}"
                );
            }
            _ => panic!("expected LoadFile call"),
        }
    }

    #[test]
    fn test_unregister_persistent_download() {
        let (player, _) = make_player();
        player.register_persistent_download(
            "123".into(),
            PathBuf::from("/tmp/downloads/123.flac"),
        );
        assert!(player.has_persistent_download("123"));
        player.unregister_persistent_download("123");
        assert!(!player.has_persistent_download("123"));
    }

    #[test]
    fn test_rehydrate_persistent_cache_replaces_contents() {
        let (player, _) = make_player();
        player.register_persistent_download(
            "old".into(),
            PathBuf::from("/tmp/downloads/old.flac"),
        );

        let mut entries = HashMap::new();
        entries.insert("new".into(), PathBuf::from("/tmp/downloads/new.flac"));
        player.rehydrate_persistent_cache(entries);

        assert!(!player.has_persistent_download("old"));
        assert!(player.has_persistent_download("new"));
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
