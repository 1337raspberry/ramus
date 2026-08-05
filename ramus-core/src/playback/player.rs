//! AudioPlayer: queue management, mpv integration, equalizer, download
//! cache. Owns the mpv handle (via `MpvPlayer` trait) and manages the
//! playback queue, track URL resolution, LRU download cache, and 10-band
//! parametric equalizer filter strings.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use url::Url;

use crate::models::{PlaybackConfig, PlaybackMode, PlaybackStatus, PlayerState, Track};
use crate::playback::download_cache::DownloadCache;
use crate::playback::mpv::{FileEndReason, LoadMode, MpvPlayer};
use crate::playback::transcode;
use crate::util::redact_urls;

/// Time after a load with no `time-pos` updates before `derive_phase` flips
/// from `Buffering` to `Stalled`. The frontend uses this to colour the row,
/// the watchdog uses it to trigger a connection re-evaluation.
pub const STALL_THRESHOLD_SECS: u64 = 12;

/// Minimum gap between two *automatic* current-track reloads (connection
/// failover or file-ended recovery). Three uncoordinated triggers — the iOS
/// network-path monitor, the stall watchdog, and prefetch's failure counter —
/// can otherwise fire back-to-back and reload the same track several times for
/// one hiccup. User-initiated seeks/`previous` bypass this (they call
/// `reload_current_track` directly and must stay responsive).
const RELOAD_COOLDOWN: Duration = Duration::from_secs(6);

/// How long after an automatic reload to treat playlist-pos churn as part of
/// the reload's insert/play/remove dance rather than a real track advance. The
/// dance re-enters the *same* index, but on platforms whose mpv reports the
/// intermediate insert shift (the playing entry momentarily moves to idx+1),
/// the pos-change arrives as a *different* index first. Suppressing events
/// until we land back on the reload index — or this window elapses — stops
/// that intermediate index being mistaken for a real advance, which would
/// snap the UI to the wrong track / 0:00 and clobber the transcode
/// `position_base`. The window is a backstop only; the common case closes it
/// the instant a pos-change lands on the reload index (well under a second).
const RELOAD_SETTLE_WINDOW: Duration = Duration::from_millis(1500);

/// 10-band EQ center frequencies in Hz.
pub const EQ_FREQUENCIES: [u32; 10] = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

/// A lookahead-window track whose audio is already on disk, paired with
/// everything the prefetch worker needs to warm its ancillary caches
/// (waveform sidecar + hero art). Only tracks with secured audio are
/// returned — we never warm the extras for something we can't play offline.
#[derive(Debug, Clone)]
pub struct WarmTarget {
    pub rating_key: String,
    pub audio_path: PathBuf,
    pub thumb: Option<String>,
}

/// Derived playback phase shown in the debug panel. Captures what mpv is
/// actually doing rather than the optimistic `PlaybackStatus` flag the rest
/// of the app uses.
///
/// `Status::Playing` flips the moment `load_queue` runs, before mpv has even
/// opened the URL — `Phase` distinguishes that from the post-`file-loaded`
/// state where time-pos is actively advancing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Stopped,
    Paused,
    /// `load_queue` ran, mpv hasn't fired `file-loaded` yet.
    Opening,
    /// mpv loaded the file but we haven't seen a position update yet.
    Buffering,
    /// position updates arriving normally.
    Playing,
    /// status is Playing but no progress for `STALL_THRESHOLD_SECS`.
    Stalled,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugInfo {
    pub source: String,
    pub resolved_url: Option<String>,
    pub server_url: Option<String>,
    pub is_remote: bool,
    pub playback_mode: PlaybackMode,
    pub queue_len: usize,
    pub queue_index: usize,
    pub lookahead_depth: u8,
    pub cached_in_lookahead: u32,
    /// Subset of `cached_in_lookahead` whose on-disk file is a transcoded
    /// (`.ogg`) prefetch — populated by `build_transcode_download_url` on
    /// the prefetch path. Caveat: a track whose original codec is also Ogg
    /// will mislabel here, but that's vanishingly rare in Plex libraries.
    pub cached_in_lookahead_transcoded: u32,
    /// Subset of `cached_in_lookahead` whose file is a direct-play copy
    /// (anything other than `.ogg`).
    pub cached_in_lookahead_direct: u32,
    pub total_in_lookahead: u32,
    pub codec: Option<String>,
    pub bitrate: Option<i32>,
    pub file_size_bytes: Option<i64>,
    /// Derived phase — preferred over `status` when reasoning about whether
    /// audio is actually flowing.
    pub phase: Phase,
    /// Seconds since the last `time-pos` event, or `None` if the current
    /// load hasn't produced one yet.
    pub seconds_since_position_update: Option<u64>,
    /// Seconds since the current track started loading.
    pub seconds_since_load: Option<u64>,
    /// Last `MPV_EVENT_END_FILE` reason seen with `Error`. Cleared on the
    /// next successful `file-loaded`. Already URL-redacted.
    pub last_load_error: Option<String>,
}

/// Allowed file extensions for cached audio files.
const ALLOWED_EXTENSIONS: &[&str] = &[
    "flac", "alac", "m4a", "mp3", "aac", "wav", "aiff", "ogg", "opus", "mp2", "bin",
];

/// Threshold in seconds: if position > this, `previous()` restarts instead of going back.
const PREVIOUS_RESTART_THRESHOLD: f64 = 3.0;

/// Build an mpv `af` lavfi equalizer filter string from gain values.
///
/// Pairs each gain with the corresponding entry from `EQ_FREQUENCIES`
/// (up to whichever is shorter). Rust's `format!` always uses `.` for
/// decimals. NaN and Inf values are sanitized to 0.0.
pub fn build_eq_filter_string(bands: &[f32]) -> String {
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
/// When EQ is enabled, returns the lavfi equalizer chain. When disabled,
/// returns an empty string — `set_audio_filters("")` interprets this as
/// "no filters", clearing anything previously set.
pub fn build_af_string(eq_enabled: bool, bands: &[f32]) -> String {
    if eq_enabled {
        build_eq_filter_string(bands)
    } else {
        String::new()
    }
}

/// Derive a `Phase` from the optimistic `PlaybackStatus` plus the load-time
/// and position-update timestamps. Encapsulated as a free function so the
/// stall watchdog can call it without holding the player lock for any
/// longer than it takes to copy three small values.
pub fn derive_phase(
    status: PlaybackStatus,
    last_position_update: Option<Instant>,
    load_started_at: Option<Instant>,
    now: Instant,
) -> Phase {
    match status {
        PlaybackStatus::Stopped => Phase::Stopped,
        PlaybackStatus::Paused => Phase::Paused,
        PlaybackStatus::Playing => match (last_position_update, load_started_at) {
            (Some(t), _) => {
                if now.saturating_duration_since(t).as_secs() >= STALL_THRESHOLD_SECS {
                    Phase::Stalled
                } else {
                    Phase::Playing
                }
            }
            (None, Some(load)) => {
                if now.saturating_duration_since(load).as_secs() >= STALL_THRESHOLD_SECS {
                    Phase::Stalled
                } else {
                    Phase::Buffering
                }
            }
            (None, None) => Phase::Opening,
        },
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
    /// Whether the device's primary network interface is cellular. Set by
    /// the platform NetworkMonitor (NWPathMonitor on iOS, ConnectivityManager
    /// on Android). Permanently `false` on desktop. Orthogonal to `is_remote`.
    is_cellular: bool,
    play_session_id: String,
    cache: DownloadCache,
    last_retried_track: Option<String>,
    /// Set by `load_queue` when the requested `start_at > 0`. mpv's first
    /// `loadfile Replace` inevitably fires `playlist-pos-change(0)` before
    /// the explicit `playlist_play_index(start_at)` lands; that transient
    /// event would otherwise be reported to Plex as a phantom track switch
    /// to queue[0]. While this is `Some(target)` and the incoming pos
    /// doesn't match, `handle_playlist_pos_change` skips state mutation.
    pending_initial_pos: Option<usize>,
    /// Wall-clock timestamp of the most recent `handle_position_change` call.
    /// Used by the stall watchdog and the debug panel to surface "no
    /// progress for N seconds" without comparing position values (which
    /// can legitimately stay 0 on a freshly loaded track).
    last_position_update: Option<Instant>,
    /// Wall-clock timestamp of the most recent track load. Set whenever the
    /// player intentionally swaps the active track (load_queue, jump,
    /// next/prev, playlist_pos_change). Lets `derive_phase` distinguish a
    /// fresh load from an established stream.
    load_started_at: Option<Instant>,
    /// Most recent unrecoverable mpv `END_FILE` error message (URL-redacted).
    /// Cleared on `file-loaded`.
    last_load_error: Option<String>,
    /// Directory mpv writes `stream-record` output to (per-track files
    /// named `<rating_key>.<ext>`). Set once at startup by the Tauri
    /// layer; left `None` makes `stream_record_option_for` return `None`
    /// so loadfile carries no per-track options. Captures the source
    /// bytes mpv pulls during playback so the spectrum analyser can
    /// process them without a second HTTP fetch.
    stream_record_dir: Option<PathBuf>,
    /// Seconds the current mpv stream is shifted from the track's true
    /// timeline. Non-zero only after a transcode `offset=` resume, where
    /// mpv sees a fresh 0-based stream that is really the track's tail;
    /// `handle_position_change` adds this so the seek bar reads correctly
    /// and `seek` subtracts it. Reset to 0 on every normal (from-the-top)
    /// load. Always 0 for direct-play, whose mpv `start=` keeps the
    /// timeline absolute.
    position_base: f64,
    /// Wall-clock of the last *automatic* current-track reload (failover or
    /// file-ended recovery). Enforces `RELOAD_COOLDOWN` so a burst of triggers
    /// can't stack multiple reloads onto one hiccup. `None` until the first.
    last_auto_reload_at: Option<Instant>,
    /// Set when recovery has been exhausted and the track is paused holding its
    /// position (rather than reset to 0:00 or skipped). While `true`, `resume`/
    /// `toggle_play_pause` re-attempt a resume-at-position load instead of a
    /// plain unpause (mpv is idle after a failed stream, so unpausing is
    /// silent). Cleared on real playback progress or a successful re-attempt.
    held_for_recovery: bool,
    /// Explicit user intent: the last play/pause-shaped user command was a
    /// pause. Owned by `pause`/`resume`/`toggle_play_pause`/`load_queue`, and
    /// recorded even when the status gate swallows the actual mpv command
    /// (e.g. a lock-screen pause while held for recovery, where status is
    /// already Paused) — automatic connection recovery consults it so it
    /// never starts audio a user asked to stop. App-internal status flips
    /// (holds, reload exits) deliberately do not touch it.
    user_paused: bool,
    /// Wall-clock of the last [`AudioPlayer::recover_interrupted_playback`]
    /// reload. Kept separate from `last_auto_reload_at`: entering a hold
    /// stamps that one, and a network that returns seconds later must not
    /// have its single recovery reload declined by the very failure it is
    /// recovering from.
    last_recovery_reload_at: Option<Instant>,
    /// Queue index of an in-flight current-track reload. The reload re-enters
    /// its own index via `playlist_play_index`, firing one or more
    /// `playlist-pos-change` events that are NOT real track advances (some
    /// platforms also report the transient insert shift at idx+1 first).
    /// `handle_playlist_pos_change` suppresses every pos-change until it lands
    /// back on this index (see [`RELOAD_SETTLE_WINDOW`]), so it never resets
    /// `position`/`position_base` to 0 or tells the platform layer to report a
    /// phantom track (re)start — otherwise the seek bar and lock screen snap to
    /// 0:00 on every failover resume. `None` except during a reload.
    reloading_pos: Option<usize>,
    /// Wall-clock when `reloading_pos` was armed. Bounds the reload settle
    /// window so a reload that never re-fires a landing pos-change (e.g. mpv
    /// coalesced the churn to a net-zero index change) can't suppress a later
    /// genuine advance indefinitely.
    reload_started_at: Option<Instant>,
    /// The track that was playing before the most recent track switch, with
    /// the position/duration it held at the moment of the switch. Recorded at
    /// mutation time by every switch path — natural advances (in
    /// `handle_playlist_pos_change`) and manual skips (`next`/`previous`/
    /// `jump_to_index`, which update `current_track` synchronously long
    /// before mpv's confirmation event arrives). Consumed by the platform
    /// layer via [`AudioPlayer::take_pending_transition`] for session
    /// reporting: by the time the pos-change callback runs, the live player
    /// already reads as the *next* track at position 0, so closing out the
    /// previous track from live state reports every finish at 0:00.
    pending_transition: Option<(Track, f64, f64)>,
}

impl PlayerInner {
    /// True if an automatic reload fired within `RELOAD_COOLDOWN`. Used to
    /// coalesce a burst of failover/recovery triggers (network-path monitor,
    /// stall watchdog, prefetch) into a single reload per hiccup.
    fn within_reload_cooldown(&self) -> bool {
        self.last_auto_reload_at
            .is_some_and(|t| t.elapsed() < RELOAD_COOLDOWN)
    }

    /// Record the currently-playing track as the outgoing side of a track
    /// switch, at its final observed position/duration. Called under the lock
    /// by every switch path *before* `current_track`/`position` are
    /// overwritten. No-op when nothing is playing.
    fn record_transition(&mut self) {
        if let Some(prev) = self.state.current_track.clone() {
            self.pending_transition = Some((prev, self.position, self.duration));
        }
    }
}

/// Outcome of a file-ended recovery attempt, returned by `handle_file_ended`
/// so the platform layer can keep the OS media controls and the in-app seek
/// bar honest about what happened (a silent reload otherwise leaves both
/// extrapolating the old position forward as if still playing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecoverOutcome {
    /// Nothing to do — a natural end-of-file, or an event we don't act on.
    None,
    /// A resume-at-position reload was issued; freeze the controls at `pos`
    /// and re-anchor on the next position tick.
    Reloading(f64),
    /// Recovery exhausted; the track is paused holding `pos` (not reset, not
    /// skipped). Reflect a paused state at `pos`.
    Held(f64),
    /// Unrecoverable (e.g. a local file that failed to decode); the player has
    /// already advanced to the next track.
    Skipped,
}

/// Snapshot of playback progress for the lock-screen now-playing keeper,
/// returned by [`AudioPlayer::media_position_snapshot`].
#[derive(Debug, Clone, Copy)]
pub struct MediaPositionSnapshot {
    /// The player believes it should be playing (status == Playing).
    pub is_playing: bool,
    /// Playing, at least one position tick has arrived for the current load,
    /// but none within the requested threshold — mid-track audio has stalled
    /// (a network hiccup or the reload gap on a failover resume). The OS
    /// scrubber must be frozen at `position` while this holds, else it keeps
    /// extrapolating forward as if still playing.
    pub progress_stalled: bool,
    /// Last known true playback position (already `position_base`-remapped).
    pub position: f64,
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
                is_cellular: false,
                play_session_id: uuid::Uuid::new_v4().to_string(),
                cache: DownloadCache::new(PlaybackConfig::DEFAULT_CACHE_LIMIT_BYTES as u64),
                last_retried_track: None,
                pending_initial_pos: None,
                last_position_update: None,
                load_started_at: None,
                last_load_error: None,
                stream_record_dir: None,
                position_base: 0.0,
                last_auto_reload_at: None,
                held_for_recovery: false,
                user_paused: false,
                last_recovery_reload_at: None,
                reloading_pos: None,
                reload_started_at: None,
                pending_transition: None,
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
    /// `should_transcode()` under `Remote` / `RemoteOrCellular`.
    pub fn is_remote(&self) -> bool {
        self.inner.lock().is_remote
    }

    /// Update only the cellular flag. Driven by the platform NetworkMonitor
    /// (NWPathMonitor on iOS, ConnectivityManager on Android). Desktop
    /// never calls this, so `is_cellular` stays `false` on those platforms.
    pub fn set_cellular(&self, is_cellular: bool) {
        self.inner.lock().is_cellular = is_cellular;
    }

    /// Configure the directory mpv writes its `stream-record` output to.
    /// Called once at startup by the Tauri layer with the audio cache
    /// path; the core can't compute this itself because it doesn't know
    /// the app's config directory layout. While set, every direct-play
    /// `loadfile` carries a `stream-record=<dir>/<rating_key>.<ext>`
    /// per-file option, so the symphonia analyser can run against the
    /// captured file without a second HTTP fetch.
    pub fn set_stream_record_dir(&self, dir: PathBuf) {
        self.inner.lock().stream_record_dir = Some(dir);
    }

    /// Read back the configured stream-record directory. Used by the
    /// prefetch worker to compute the on-disk path for an ingest pass.
    pub fn stream_record_dir(&self) -> Option<PathBuf> {
        self.inner.lock().stream_record_dir.clone()
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

        // Snapshot the per-track (url, stream-record options) pairs under
        // the lock, then release before touching mpv (FFI calls may block
        // briefly).
        let loads: Vec<Option<(String, Option<String>)>> = {
            let persistent = self.persistent_cache.read();
            let mut inner = self.inner.lock();
            inner.state.queue = tracks;
            inner.state.queue_index = start_at;
            inner.state.current_track = Some(inner.state.queue[start_at].clone());
            inner.state.status = PlaybackStatus::Playing;
            // Explicit fresh play: any earlier pause intent is superseded
            // (the trailing set_pause(false) makes it real at the mpv level),
            // and a lingering recovery hold is moot — it must be released
            // here or the new queue's pos-change events would be suppressed
            // as the hold's auto-advance walk.
            inner.user_paused = false;
            inner.held_for_recovery = false;
            // A fresh queue also moots any in-flight current-track reload —
            // close the settle window (like next/previous/jump do) so an
            // armed window can't swallow the new queue's first pos-change
            // as the old reload's landing event.
            inner.reloading_pos = None;
            inner.reload_started_at = None;
            // The caller reports the old queue's close-out itself (with the
            // player still in its pre-load state); a leftover snapshot must
            // not fire on the new queue's first pos-change and re-report a
            // track from the previous session.
            inner.pending_transition = None;
            inner.play_session_id = uuid::Uuid::new_v4().to_string();
            inner.position = 0.0;
            inner.position_base = 0.0;
            // Seed from Plex metadata. mpv's reported duration for our
            // chunked-Opus transcode stream (no Content-Length) flutters
            // as it reads ahead — the metadata value is stable and the
            // user already trusts it everywhere else in the UI.
            inner.duration = inner.state.queue[start_at].duration;
            inner.is_loading = false;
            inner.load_started_at = Some(Instant::now());
            inner.last_position_update = None;
            inner.last_load_error = None;
            // Suppress the transient pos=0 event mpv fires from the first
            // loadfile Replace before our playlist_play_index(start_at) call.
            inner.pending_initial_pos = if start_at > 0 { Some(start_at) } else { None };

            inner
                .state
                .queue
                .iter()
                .map(|t| {
                    resolve_url(t, &inner, &persistent).map(|url| {
                        let opts = stream_record_option_for(t, &url, &inner);
                        (url, opts)
                    })
                })
                .collect()
        };

        for (i, load) in loads.iter().enumerate() {
            if let Some((url, opts)) = load {
                let mode = if i == 0 {
                    LoadMode::Replace
                } else {
                    LoadMode::Append
                };
                // Track URLs contain `X-Plex-Token` in the query string —
                // log only enough to correlate with mpv events, never the
                // URL itself. Stream-record paths are token-free.
                log::debug!("load_queue[{i}]: mode={mode:?} stream_record={}", opts.is_some());
                self.mpv.load_file(url, mode, opts.as_deref());
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
                let loads: Vec<Option<(String, Option<String>)>> = tracks
                    .iter()
                    .map(|t| {
                        resolve_url(t, &inner, &persistent).map(|url| {
                            let opts = stream_record_option_for(t, &url, &inner);
                            (url, opts)
                        })
                    })
                    .collect();
                (false, loads)
            }
        };

        if was_stopped {
            let queue = self.inner.lock().state.queue.clone();
            self.load_queue(queue, 0);
        } else {
            for (url, opts) in loads.into_iter().flatten() {
                self.mpv.load_file(&url, LoadMode::Append, opts.as_deref());
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

            let loads: Vec<Option<(String, Option<String>)>> = tracks
                .iter()
                .map(|t| {
                    resolve_url(t, &inner, &persistent).map(|url| {
                        let opts = stream_record_option_for(t, &url, &inner);
                        (url, opts)
                    })
                })
                .collect();
            (insert_base, loads)
        };

        for (offset, load) in loads.iter().enumerate() {
            if let Some((url, opts)) = load {
                self.mpv.load_file_at(
                    url,
                    (insert_base + offset) as i64,
                    opts.as_deref(),
                );
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

        // Explicit track selection is unconditional play intent: it
        // releases any recovery hold (before commanding mpv, so the
        // confirmation pos-change isn't suppressed as the walk),
        // supersedes any pause intent, and makes the start real at the
        // mpv level — mpv's sticky pause (a user pause, or the hold's
        // pin) would otherwise leave the selected track silent under the
        // Playing status set below.
        inner.record_transition();
        inner.held_for_recovery = false;
        inner.user_paused = false;
        inner.state.queue_index = index;
        inner.state.current_track = Some(inner.state.queue[index].clone());
        inner.position = 0.0;
        inner.duration = inner.state.queue[index].duration;
        inner.state.status = PlaybackStatus::Playing;
        inner.load_started_at = Some(Instant::now());
        inner.last_position_update = None;
        inner.last_load_error = None;
        // A deliberate track switch moots any in-flight current-track
        // reload: close the settle window and drop the offset base so a
        // suppressed pos-change can't leave them applied to the new track.
        inner.reloading_pos = None;
        inner.reload_started_at = None;
        inner.position_base = 0.0;
        drop(inner);
        self.mpv.set_pause(false);
        self.mpv.playlist_play_index(index as i64);
    }

    /// Advance to the next track. Stops if at the end of the queue.
    pub fn next(&self) {
        let mut inner = self.inner.lock();
        if inner.state.queue_index + 1 >= inner.state.queue.len() {
            // Skipping off the end still ends the current track at a real
            // position — the idle callback consumes the snapshot (there is
            // no pos-change event on this path, mpv just goes idle).
            inner.record_transition();
            inner.state.status = PlaybackStatus::Stopped;
            inner.state.current_track = None;
            inner.position = 0.0;
            inner.load_started_at = None;
            inner.last_position_update = None;
            inner.held_for_recovery = false;
            drop(inner);
            self.mpv.stop();
            return;
        }

        // Deliberate navigation releases a recovery hold BEFORE commanding
        // mpv, so the confirmation pos-change isn't suppressed as the
        // auto-advance walk. The hold parked mpv paused; a skip with play
        // intent must lift that pin or the next track loads silent, while
        // an explicitly paused user keeps their pause (sticky across the
        // index change).
        inner.record_transition();
        let unpause_after = inner.held_for_recovery && !inner.user_paused;
        inner.held_for_recovery = false;
        if unpause_after {
            inner.state.status = PlaybackStatus::Playing;
        }
        inner.state.queue_index += 1;
        inner.state.current_track = Some(inner.state.queue[inner.state.queue_index].clone());
        inner.position = 0.0;
        inner.duration = inner.state.queue[inner.state.queue_index].duration;
        inner.load_started_at = Some(Instant::now());
        inner.last_position_update = None;
        inner.last_load_error = None;
        // A deliberate track switch moots any in-flight current-track
        // reload: close the settle window and drop the offset base so a
        // suppressed pos-change can't leave them applied to the new track.
        inner.reloading_pos = None;
        inner.reload_started_at = None;
        inner.position_base = 0.0;
        let idx = inner.state.queue_index;
        drop(inner);
        if unpause_after {
            self.mpv.set_pause(false);
        }
        self.mpv.playlist_play_index(idx as i64);
    }

    /// Go to the previous track, or restart the current track if > 3s in.
    pub fn previous(&self) {
        let mut inner = self.inner.lock();

        if inner.position > PREVIOUS_RESTART_THRESHOLD {
            // A held track has no live mpv stream — a plain seek(0) would
            // be a silent no-op. Reload from the top instead (the hold
            // exit inside restores the pause flag to the user's intent).
            // A transcode `offset=` stream can't rewind before its resume
            // point either; same fresh reload.
            if inner.held_for_recovery || inner.position_base > 0.0 {
                let idx = inner.state.queue_index;
                drop(inner);
                self.reload_current_track(None, Some(idx));
                return;
            }
            inner.position = 0.0;
            drop(inner);
            self.mpv.seek(0.0);
            return;
        }

        if inner.state.queue_index == 0 {
            // Same restart-current outcome as above, reached via the other
            // threshold — held and offset streams can't seek here either.
            if inner.held_for_recovery || inner.position_base > 0.0 {
                drop(inner);
                self.reload_current_track(None, Some(0));
                return;
            }
            inner.position = 0.0;
            drop(inner);
            self.mpv.seek(0.0);
            return;
        }

        // See `next`: release a recovery hold before commanding mpv, and
        // lift its pause pin when the user wants playback.
        inner.record_transition();
        let unpause_after = inner.held_for_recovery && !inner.user_paused;
        inner.held_for_recovery = false;
        if unpause_after {
            inner.state.status = PlaybackStatus::Playing;
        }
        inner.state.queue_index -= 1;
        inner.state.current_track = Some(inner.state.queue[inner.state.queue_index].clone());
        inner.position = 0.0;
        inner.duration = inner.state.queue[inner.state.queue_index].duration;
        inner.load_started_at = Some(Instant::now());
        inner.last_position_update = None;
        inner.last_load_error = None;
        // See `next`: a deliberate switch closes the reload settle window.
        inner.reloading_pos = None;
        inner.reload_started_at = None;
        inner.position_base = 0.0;
        let idx = inner.state.queue_index;
        drop(inner);
        if unpause_after {
            self.mpv.set_pause(false);
        }
        self.mpv.playlist_play_index(idx as i64);
    }

    /// Toggle between playing and paused.
    pub fn toggle_play_pause(&self) {
        // A track held after exhausted recovery re-attempts the resume rather
        // than toggling a dead (idle) stream.
        if self.inner.lock().held_for_recovery {
            self.resume();
            return;
        }
        let mut inner = self.inner.lock();
        match inner.state.status {
            PlaybackStatus::Playing => {
                inner.user_paused = true;
                inner.state.status = PlaybackStatus::Paused;
                drop(inner);
                self.mpv.set_pause(true);
            }
            PlaybackStatus::Paused => {
                inner.user_paused = false;
                inner.state.status = PlaybackStatus::Playing;
                inner.last_position_update = Some(Instant::now());
                drop(inner);
                self.mpv.set_pause(false);
            }
            PlaybackStatus::Stopped => {}
        }
    }

    /// Unconditionally pause playback. Safe to call when already paused.
    pub fn pause(&self) {
        let mut inner = self.inner.lock();
        // Record the intent even when the status gate below declines the
        // mpv command (e.g. a hold already shows Paused): automatic
        // connection recovery must know the user asked for silence.
        inner.user_paused = true;
        if inner.state.status == PlaybackStatus::Playing {
            inner.state.status = PlaybackStatus::Paused;
            drop(inner);
            self.mpv.set_pause(true);
        }
    }

    /// Unconditionally resume playback. Safe to call when already playing.
    pub fn resume(&self) {
        let mut inner = self.inner.lock();
        inner.user_paused = false;
        if inner.held_for_recovery {
            // The stream died and we're holding at position; mpv is idle so a
            // plain unpause is silent. Re-attempt a resume-at-position load.
            // reload_current_track clears the hold and flips status to
            // Playing only when it actually issues a load — a declined
            // reload keeps holding instead of claiming Playing over an
            // idle mpv.
            inner.last_retried_track = None;
            inner.last_auto_reload_at = None;
            let resume = inner.position;
            let idx = inner.state.queue_index;
            drop(inner);
            self.reload_current_track(Some(resume), Some(idx));
            return;
        }
        if inner.state.status == PlaybackStatus::Paused {
            inner.state.status = PlaybackStatus::Playing;
            inner.last_position_update = Some(Instant::now());
            drop(inner);
            self.mpv.set_pause(false);
        }
    }

    /// Seek to an absolute position in seconds.
    pub fn seek(&self, position: f64) {
        let mut inner = self.inner.lock();
        let clamped = position.max(0.0).min((inner.duration - 0.5).max(0.0));
        let idx = inner.state.queue_index;
        // A track held after exhausted recovery has no live stream — a plain
        // mpv seek would be a silent no-op. Re-attempt the load at the drag
        // target instead (reload_current_track exits the hold).
        if inner.held_for_recovery {
            drop(inner);
            self.reload_current_track(Some(clamped), Some(idx));
            return;
        }
        let base = inner.position_base;
        // On a transcode `offset=` stream mpv can only reach [base, end];
        // seeking before the resume point needs a fresh transcode from the
        // target, so reload rather than seek. `base` is 0 for direct-play
        // and local files, so this never fires for them.
        if base > 0.0 && clamped < base {
            drop(inner);
            self.reload_current_track(Some(clamped), Some(idx));
            return;
        }
        inner.position = clamped;
        drop(inner);
        // For a transcode offset stream mpv's 0 maps to `base` on the
        // track timeline, so translate before issuing the seek.
        self.mpv.seek(clamped - base);
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
        inner.position_base = 0.0;
        inner.duration = 0.0;
        inner.load_started_at = None;
        inner.last_position_update = None;
        inner.last_load_error = None;
        inner.held_for_recovery = false;
        inner.user_paused = false;
        inner.last_auto_reload_at = None;
        inner.last_recovery_reload_at = None;
        inner.reloading_pos = None;
        inner.reload_started_at = None;
        inner.pending_transition = None;
        drop(inner);
        self.mpv.stop();
    }

    /// Apply or clear the equalizer. When `enabled` is false the `af`
    /// chain is cleared entirely.
    pub fn apply_equalizer(&self, enabled: bool, bands: &[f32]) {
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

    /// Consume the pending track-switch snapshot: the track that was playing
    /// before the most recent switch, with its final `(position, duration)`.
    /// The platform layer calls this from the pos-change and idle callbacks
    /// to close out the previous track's Plex session at its true position —
    /// by the time those callbacks run, the live player already reads as the
    /// next track at 0:00. `None` when the last pos-change was a queue
    /// reload (the caller reported that itself) or nothing was playing.
    pub fn take_pending_transition(&self) -> Option<(Track, f64, f64)> {
        self.inner.lock().pending_transition.take()
    }

    pub fn debug_snapshot(&self) -> DebugInfo {
        let persistent = self.persistent_cache.read();
        let inner = self.inner.lock();
        let track = inner.state.queue.get(inner.state.queue_index);

        let (source, resolved_url) = match track {
            Some(t) => {
                if persistent.contains_key(&t.rating_key) {
                    ("downloaded".into(), persistent.get(&t.rating_key)
                        .map(|p| format!("file://{}", p.display())))
                } else if let Some(path) = inner.cache.get(&t.rating_key) {
                    ("cached".into(), Some(format!("file://{}", path.display())))
                } else if transcode::should_transcode(
                    t.codec.as_deref(),
                    inner.config.playback_mode,
                    inner.is_remote,
                    inner.is_cellular,
                ) {
                    ("transcode".into(), inner.server_url.as_ref().map(|u| {
                        format!("{}/audio/:/transcode/…", u.as_str().trim_end_matches('/'))
                    }))
                } else {
                    ("streaming".into(), t.part_key.as_ref().and_then(|pk| {
                        inner.server_url.as_ref().map(|u| {
                            format!("{}{}", u.as_str().trim_end_matches('/'), pk)
                        })
                    }))
                }
            }
            None => ("none".into(), None),
        };

        let depth = inner.config.lookahead_depth as usize;
        let pos = inner.state.queue_index;
        let mut cached_in_lookahead = 0u32;
        let mut cached_in_lookahead_transcoded = 0u32;
        let mut cached_in_lookahead_direct = 0u32;
        let mut total_in_lookahead = 0u32;
        for offset in 1..=depth {
            let Some(t) = inner.state.queue.get(pos + offset) else {
                break;
            };
            total_in_lookahead += 1;
            let path = persistent
                .get(&t.rating_key)
                .cloned()
                .or_else(|| inner.cache.get(&t.rating_key).map(|p| p.to_path_buf()));
            if let Some(p) = path {
                cached_in_lookahead += 1;
                let is_transcoded = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("ogg"))
                    .unwrap_or(false);
                if is_transcoded {
                    cached_in_lookahead_transcoded += 1;
                } else {
                    cached_in_lookahead_direct += 1;
                }
            }
        }

        let now = Instant::now();
        let phase = derive_phase(
            inner.state.status,
            inner.last_position_update,
            inner.load_started_at,
            now,
        );

        DebugInfo {
            source,
            resolved_url,
            server_url: inner.server_url.as_ref().map(|u| u.to_string()),
            is_remote: inner.is_remote,
            playback_mode: inner.config.playback_mode,
            queue_len: inner.state.queue.len(),
            queue_index: inner.state.queue_index,
            lookahead_depth: inner.config.lookahead_depth,
            cached_in_lookahead,
            cached_in_lookahead_transcoded,
            cached_in_lookahead_direct,
            total_in_lookahead,
            codec: track.and_then(|t| t.codec.clone()),
            bitrate: track.and_then(|t| t.bitrate),
            file_size_bytes: track.and_then(|t| t.file_size_bytes),
            phase,
            seconds_since_position_update: inner
                .last_position_update
                .map(|t| now.saturating_duration_since(t).as_secs()),
            seconds_since_load: inner
                .load_started_at
                .map(|t| now.saturating_duration_since(t).as_secs()),
            last_load_error: inner.last_load_error.clone(),
        }
    }

    /// Snapshot the current playback progress for the lock-screen now-playing
    /// keeper. `stall_threshold` is how long without a `time-pos` tick (while
    /// Playing, after at least one tick has arrived) counts as a mid-track
    /// stall. Distinct from [`is_stalled`], which uses the longer
    /// connection-recovery threshold and also treats pre-first-tick buffering
    /// as a stall.
    pub fn media_position_snapshot(&self, stall_threshold: Duration) -> MediaPositionSnapshot {
        let inner = self.inner.lock();
        let is_playing = inner.state.status == PlaybackStatus::Playing;
        let progress_stalled = is_playing
            && inner
                .last_position_update
                .is_some_and(|t| t.elapsed() >= stall_threshold);
        MediaPositionSnapshot {
            is_playing,
            progress_stalled,
            position: inner.position,
        }
    }

    /// Whether mpv has had no `time-pos` activity for `STALL_THRESHOLD_SECS`
    /// while we believe it should be playing. Used by the stall watchdog
    /// to fire a connection re-evaluation (the only thing that might
    /// recover a stuck transcode session against an unreachable host).
    pub fn is_stalled(&self) -> bool {
        let inner = self.inner.lock();
        derive_phase(
            inner.state.status,
            inner.last_position_update,
            inner.load_started_at,
            Instant::now(),
        ) == Phase::Stalled
    }

    /// Whether playback sits in an app-inflicted silent state that a healthy
    /// connection could fix: held for recovery, or Playing with no progress
    /// (a dead socket after a silent network flip never errors — mpv just
    /// stops making progress). False when the user explicitly paused; their
    /// intent outranks recovery. Drives the stall watchdog's trigger.
    pub fn needs_connection_recovery(&self) -> bool {
        let inner = self.inner.lock();
        if inner.user_paused {
            return false;
        }
        inner.held_for_recovery
            || derive_phase(
                inner.state.status,
                inner.last_position_update,
                inner.load_started_at,
                Instant::now(),
            ) == Phase::Stalled
    }

    /// Re-attempt playback after the connection layer reports the server
    /// healthy again. An `Unchanged` evaluation fires no callback — for a
    /// remote/cloud server that is the *only* shape recovery ever takes, so
    /// the recovered-edge handler and the stall watchdog both funnel that
    /// verdict here. Reloads the current track at position when playback is
    /// held for recovery or stalled; declines when the user explicitly
    /// paused (recovery must never start audio the user asked to stop).
    /// Returns `true` if a reload was issued.
    pub fn recover_interrupted_playback(&self) -> bool {
        let (resume, idx) = {
            let mut inner = self.inner.lock();
            if inner.user_paused {
                return false;
            }
            let stalled = derive_phase(
                inner.state.status,
                inner.last_position_update,
                inner.load_started_at,
                Instant::now(),
            ) == Phase::Stalled;
            if !inner.held_for_recovery && !stalled {
                return false;
            }
            // One recovery reload per episode: racing triggers (path-event
            // recovery edge + watchdog verdict) coalesce here. Deliberately
            // NOT `within_reload_cooldown()` — entering the hold stamped
            // `last_auto_reload_at`, and a network that returns seconds
            // later must not have its recovery declined by the very failure
            // it is recovering from.
            if inner
                .last_recovery_reload_at
                .is_some_and(|t| t.elapsed() < RELOAD_COOLDOWN)
            {
                return false;
            }
            inner.last_recovery_reload_at = Some(Instant::now());
            inner.last_auto_reload_at = Some(Instant::now());
            // The connection just changed state; a retry verdict from
            // before the recovery is no longer informative. Granting a
            // fresh attempt also re-arms the organic failure ladder for
            // the reloaded stream.
            inner.last_retried_track = None;
            (inner.position, inner.state.queue_index)
        };
        self.reload_current_track(Some(resume), Some(idx))
    }

    /// Handle mpv position change (called by event loop, ~30fps).
    pub fn handle_position_change(&self, pos: f64) {
        let mut inner = self.inner.lock();
        // `position_base` is non-zero while a transcode `offset=` resume
        // stream plays (mpv reports 0-based; the real position is shifted
        // by the resume point).
        inner.position = pos + inner.position_base;
        inner.last_position_update = Some(Instant::now());
        // Audio is actually flowing — clear the retry guard so a *later*
        // failure on this same track (e.g. a network blip mid-song) can
        // get a fresh retry. Was previously cleared in `handle_file_loaded`
        // but mpv fires file-loaded the moment the URL opens, BEFORE
        // knowing whether the body will succeed; on slow-connection
        // single-file transcode failures we'd see file-loaded → file-ended
        // → retry → file-loaded → file-ended → retry forever, never
        // converging because the guard kept clearing.
        inner.last_retried_track = None;
        // Audio is flowing again — a recovery reload (from any trigger)
        // succeeded, so clear the hold guard even if it wasn't `resume` that
        // re-attempted it.
        inner.held_for_recovery = false;
    }

    /// Handle mpv duration change.
    ///
    /// Only accepted when we don't already have a non-zero duration —
    /// every track-load site seeds duration from Plex metadata, which is
    /// stable across UI ticks. mpv's reported duration for our chunked
    /// Opus transcode stream (no Content-Length) flutters as the demuxer
    /// reads ahead, so accepting it would cause the seek-bar percentage
    /// to jump. Falls back to mpv's value only when metadata is missing.
    pub fn handle_duration_change(&self, dur: f64) {
        let mut inner = self.inner.lock();
        if inner.duration <= 0.0 && dur > 0.0 {
            inner.duration = dur;
        }
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
    /// Returns `true` if this was a real track advance (the platform layer
    /// should emit a track-switch + refresh now-playing metadata), or `false`
    /// for a transient/self-inflicted pos-change — an invalid index, the
    /// phantom pos=0 during a `start_at` load, or our own current-track reload
    /// — which must NOT be reported as a (re)start (that snaps the seek bar and
    /// lock screen to 0:00).
    #[must_use]
    pub fn handle_playlist_pos_change(&self, pos: i64) -> bool {
        if pos < 0 {
            return false;
        }
        let mut inner = self.inner.lock();
        let pos = pos as usize;
        if pos >= inner.state.queue.len() {
            // An insert-at during a current-track reload transiently shifts the
            // playing entry to queue.len() (mpv's playlist momentarily holds
            // the extra entry). That is never a real advance either.
            return false;
        }

        // While held for recovery, every mpv-originated pos-change is the
        // auto-advance walk (keep-open=no moves past the failed entry and
        // through the playlist), not a real advance — the hold owns the
        // queue position. Processing the walk used to clear the hold and
        // cascade the queue pointer through the outage. Deliberate
        // navigation is unaffected: next/previous/jump and load_queue clear
        // the hold under their own lock BEFORE commanding mpv, so their
        // confirmation events arrive with the hold already released.
        if inner.held_for_recovery {
            log::debug!("playlist_pos_change({pos}) suppressed: held for recovery");
            return false;
        }

        // A current-track reload (failover/recovery resume, or a seek that
        // re-opens a transcode offset stream) re-enters its own index via an
        // insert/play/remove dance. That dance can fire *several*
        // playlist-pos-change events — including a transient one at the
        // insert-shifted index (idx+1) before it settles back on the reload
        // index. None are track changes: suppress every event until we land on
        // the reload index (which closes the window) or the settle window
        // elapses. Reporting any of them as an advance would reset the resume
        // position/base to 0 and make the platform layer snap the UI to the
        // wrong track / 0:00.
        if let Some(reload_idx) = inner.reloading_pos {
            let expired = inner
                .reload_started_at
                .is_none_or(|t| t.elapsed() >= RELOAD_SETTLE_WINDOW);
            if pos == reload_idx {
                // Landed — the reload has settled on its own index.
                inner.reloading_pos = None;
                inner.reload_started_at = None;
                return false;
            }
            if !expired {
                // Transient churn from the dance (e.g. the insert shift) —
                // keep the window open and wait for the landing event.
                return false;
            }
            // The window elapsed without a landing event (mpv coalesced the
            // churn to a net-zero index change, or the reload failed over to a
            // genuine advance). Clear and fall through to normal handling.
            inner.reloading_pos = None;
            inner.reload_started_at = None;
        }

        // Drop the transient pos=0 event mpv fires from the first loadfile
        // Replace during a load_queue with start_at > 0. Without this guard,
        // the lib.rs callback would observe current_track flipping briefly
        // to queue[0] and emit a phantom track-switch session report to Plex.
        if let Some(target) = inner.pending_initial_pos {
            if pos != target {
                return false;
            }
            inner.pending_initial_pos = None;
        }

        // A genuine track change records the outgoing track at its final
        // position for session reporting. Same rating key means this event
        // only *confirms* a manual skip that already mutated state (and
        // already recorded the transition with the true pre-skip position —
        // recording again here would clobber it with the zeroed one).
        let is_new_track = inner
            .state
            .current_track
            .as_ref()
            .is_none_or(|cur| cur.rating_key != inner.state.queue[pos].rating_key);
        if is_new_track {
            inner.record_transition();
        }

        inner.state.queue_index = pos;
        inner.state.current_track = Some(inner.state.queue[pos].clone());
        inner.position = 0.0;
        inner.position_base = 0.0;
        // Reseed duration from metadata on every gapless advance — see
        // load_queue's note. Stable across UI ticks regardless of mpv's
        // streamed-source duration estimation.
        inner.duration = inner.state.queue[pos].duration;
        inner.load_started_at = Some(Instant::now());
        inner.last_position_update = None;
        inner.last_load_error = None;
        inner.last_retried_track = None;
        true
    }

    /// Handle mpv pause state change.
    pub fn handle_pause_change(&self, paused: bool) {
        let mut inner = self.inner.lock();
        if paused && inner.state.status == PlaybackStatus::Playing {
            inner.state.status = PlaybackStatus::Paused;
        } else if !paused && inner.state.status == PlaybackStatus::Paused {
            inner.state.status = PlaybackStatus::Playing;
            // Reset the progress timer so a long pause doesn't make the
            // watchdog think we just stalled the moment we resume. The
            // first real `time-pos` from mpv will overwrite this within
            // ~50ms.
            inner.last_position_update = Some(Instant::now());
        }
    }

    /// Rewrite all non-cached, non-current mpv playlist entries to use the
    /// current `server_url` and `token`. Called after connection failover
    /// so stale URLs don't cascade-fail when playback reaches them.
    pub fn rewrite_stale_playlist_urls(&self) {
        let rewrites: Vec<(usize, String, Option<String>)> = {
            let persistent = self.persistent_cache.read();
            let inner = self.inner.lock();
            let current_idx = inner.state.queue_index;

            inner
                .state
                .queue
                .iter()
                .enumerate()
                .filter_map(|(idx, track)| {
                    if idx == current_idx {
                        return None;
                    }
                    if persistent.contains_key(&track.rating_key) {
                        return None;
                    }
                    if inner.cache.get(&track.rating_key).is_some() {
                        return None;
                    }
                    let url = resolve_url(track, &inner, &persistent)?;
                    if url.starts_with("file://") {
                        return None;
                    }
                    let opts = stream_record_option_for(track, &url, &inner);
                    Some((idx, url, opts))
                })
                .collect()
        };

        if rewrites.is_empty() {
            return;
        }

        log::info!(
            "rewriting {} stale playlist entries after connection change",
            rewrites.len()
        );

        for (idx, new_url, opts) in rewrites.iter().rev() {
            self.mpv.playlist_remove(*idx as i64);
            self.mpv.load_file_at(new_url, *idx as i64, opts.as_deref());
        }
    }

    /// Handle mpv file-loaded event.
    pub fn handle_file_loaded(&self) {
        let mut inner = self.inner.lock();
        inner.is_loading = false;
        // NOTE: deliberately do NOT clear `last_retried_track` here. mpv
        // fires file-loaded as soon as the URL opens — the body may still
        // die mid-stream. The retry guard is cleared in
        // `handle_position_change` once audio is actually flowing.
        inner.last_load_error = None;
    }

    /// Handle mpv file-ended event. Returns a [`RecoverOutcome`] the platform
    /// layer uses to keep the OS media controls and seek bar in sync.
    pub fn handle_file_ended(&self, reason: FileEndReason) -> RecoverOutcome {
        match reason {
            FileEndReason::Eof => {
                // Natural end — mpv auto-advances via gapless playback;
                // if last track, idle-active will fire.
                RecoverOutcome::None
            }
            FileEndReason::Error(ref msg) => {
                let redacted = redact_urls(msg);
                self.inner.lock().last_load_error = Some(redacted.clone());
                match self.try_recover_current_track() {
                    RecoverOutcome::Reloading(pos) => {
                        log::warn!("handle_file_ended: load error, resuming at {pos:.1}s: {redacted}");
                        RecoverOutcome::Reloading(pos)
                    }
                    RecoverOutcome::Held(pos) => {
                        // Recovery exhausted (already retried, or within the
                        // reload cooldown). Hold at position instead of
                        // resetting to 0:00 or skipping — a play tap
                        // re-attempts the resume (see `resume`).
                        {
                            let mut inner = self.inner.lock();
                            inner.state.status = PlaybackStatus::Paused;
                            inner.held_for_recovery = true;
                        }
                        // Pin mpv: with keep-open=no it advances past the
                        // failed entry on its own and walks the rest of the
                        // playlist. The sticky pause keeps that walk silent —
                        // without it, the first entry that *does* load (a
                        // cached file, say) starts playing audibly under a
                        // Paused status. The walk's pos-changes and any
                        // eventual idle are suppressed separately (see
                        // `handle_playlist_pos_change` / `handle_idle_active`);
                        // every hold exit restores the pause flag to the
                        // user's actual intent.
                        self.mpv.set_pause(true);
                        log::warn!("handle_file_ended: recovery exhausted, holding at {pos:.1}s: {redacted}");
                        RecoverOutcome::Held(pos)
                    }
                    _ => {
                        // Only a genuinely local/undownloadable track reaches
                        // here — skipping is the sole sensible recovery.
                        log::warn!("handle_file_ended: unrecoverable error, skipping: {redacted}");
                        self.next();
                        RecoverOutcome::Skipped
                    }
                }
            }
            _ => RecoverOutcome::None,
        }
    }

    /// Attempt to recover from a failed track load by resuming it at the last
    /// known position over the current server connection. A network track's
    /// first failure resumes ([`RecoverOutcome::Reloading`]); a second failure
    /// on the same track, or one inside the reload cooldown, holds at position
    /// ([`RecoverOutcome::Held`]) rather than thrash or reset. A local file
    /// that failed to decode yields [`RecoverOutcome::Skipped`] via the caller.
    fn try_recover_current_track(&self) -> RecoverOutcome {
        // Capture, guard, and stamp in ONE lock window so idx/track/resume
        // stay consistent; `expected_idx` then lets reload_current_track
        // decline if a user skip lands before it re-acquires the lock.
        let (resume, idx) = {
            let persistent = self.persistent_cache.read();
            let mut inner = self.inner.lock();
            let idx = inner.state.queue_index;
            let Some(track) = inner.state.queue.get(idx) else {
                return RecoverOutcome::Skipped;
            };
            let rating_key = track.rating_key.clone();
            // Local files: a file-ended error is a genuine decode/file problem,
            // not a transient stream drop — let the caller skip.
            if persistent.contains_key(&rating_key) || inner.cache.get(&rating_key).is_some() {
                return RecoverOutcome::Skipped;
            }
            // Network stream. Hold (don't thrash/reset) if we already retried
            // this track, or if the last automatic reload was too recent.
            if inner.last_retried_track.as_deref() == Some(rating_key.as_str())
                || inner.within_reload_cooldown()
            {
                return RecoverOutcome::Held(inner.position);
            }
            inner.last_retried_track = Some(rating_key);
            inner.last_auto_reload_at = Some(Instant::now());
            (inner.position, idx)
        };
        // Resume at the captured position (transcode `offset=` / direct-play
        // `start=` per `reload_current_track`), never a restart from 0:00.
        if self.reload_current_track(Some(resume), Some(idx)) {
            RecoverOutcome::Reloading(resume)
        } else {
            // Reload declined (became file:// or stopped meanwhile) — hold
            // rather than reset; there's nothing safe to skip to.
            RecoverOutcome::Held(resume)
        }
    }

    /// Reload the current track's mpv entry, optionally resuming at
    /// `resume` seconds. Shared by the connection-failover reload
    /// ([`force_reload_current_track`]) and the seek/restart paths that
    /// must re-open a transcode `offset=` stream from a new point. Returns
    /// `true` if a reload was issued. Skips stopped, LRU-cached, and
    /// persistent-download (`file://`) tracks — their local URL is
    /// unaffected by a server change and mpv can seek them directly.
    ///
    /// Direct-play/local resume is an mpv `start=` seek (timeline stays
    /// absolute); a transcode resume is a server-side `offset=` with
    /// `position_base` remapping mpv's 0-based stream onto the track
    /// timeline. A transcode reload with no resume (or a resume the server
    /// rejects → file-ended → `try_recover_current_track`) simply starts
    /// from the top — the graceful fallback so a refused resume never
    /// skips the track.
    fn reload_current_track(&self, resume: Option<f64>, expected_idx: Option<usize>) -> bool {
        let (idx, url, opts, was_held, stay_paused) = {
            let persistent = self.persistent_cache.read();
            let mut inner = self.inner.lock();
            if inner.state.status == PlaybackStatus::Stopped {
                return false;
            }
            let idx = inner.state.queue_index;
            // The caller captured its resume position under an earlier lock
            // acquisition; if a skip/jump landed in between, that position
            // belongs to a different track — decline rather than resume the
            // wrong track at the old track's position.
            if expected_idx.is_some_and(|e| e != idx) {
                return false;
            }
            let Some(track) = inner.state.queue.get(idx).cloned() else {
                return false;
            };
            if persistent.contains_key(&track.rating_key) {
                return false;
            }
            if inner.cache.get(&track.rating_key).is_some() {
                return false;
            }
            let Some((url, plan)) =
                resolve_url_with_resume(&track, &inner, &persistent, resume)
            else {
                return false;
            };
            if url.starts_with("file://") {
                return false;
            }
            let (opts, base) = match plan {
                ResumePlan::None => (None, 0.0),
                ResumePlan::MpvSeek(secs) => (Some(format!("start={secs:.3}")), 0.0),
                ResumePlan::StreamOffset(secs) => (None, secs),
            };
            inner.position_base = base;
            // Reflect the resume point immediately so the seek bar doesn't
            // flash back to 0:00 before mpv's first position event lands.
            // For a stream-offset resume use the truncated base the server
            // was actually asked for, so the first real tick doesn't step
            // backwards by the sub-second remainder.
            if base > 0.0 {
                inner.position = base;
            } else if let Some(r) = resume.filter(|p| *p > 0.0) {
                inner.position = r;
            }
            // Exiting a hold: entering it parked mpv paused (see the Held
            // arm of `handle_file_ended`), so the pause flag must be
            // restored to the user's actual intent after the load below —
            // `set_pause(false)` when they want playback (the load then
            // genuinely starts audio, so flip the logical status to match),
            // or kept paused when they explicitly paused during the outage
            // (the track reloads at position, silent, ready for their play
            // tap — status stays Paused and truthful). An ordinary
            // non-held reload is unaffected: mpv's sticky pause carries
            // across loadfile, and `was_held` is false so neither the
            // status flip nor the pause command fires.
            let was_held = inner.held_for_recovery;
            let stay_paused = inner.user_paused;
            if was_held {
                inner.held_for_recovery = false;
                if !stay_paused {
                    inner.state.status = PlaybackStatus::Playing;
                    inner.last_position_update = Some(Instant::now());
                }
            }
            // The play-index below re-enters this index and fires one or more
            // playlist-pos-change events; arm the settle window so
            // `handle_playlist_pos_change` treats them as a reload, not a track
            // advance (preserving the resume position/base instead of zeroing
            // them), including any transient insert-shift index.
            inner.reloading_pos = Some(idx);
            inner.reload_started_at = Some(Instant::now());
            (idx, url, opts, was_held, stay_paused)
        };

        log::info!("reloading current track (resume={resume:?})");
        // Insert/play/remove dance — can't playlist_remove the active
        // index (mpv may still hold it), so insert fresh before it, play
        // the fresh entry, then remove the stale one shifted to idx+1.
        self.mpv.load_file_at(&url, idx as i64, opts.as_deref());
        self.mpv.playlist_play_index(idx as i64);
        self.mpv.playlist_remove((idx + 1) as i64);
        if was_held {
            // Restore the pause flag from the hold's pin to the user's
            // intent (see the hold-exit note above).
            self.mpv.set_pause(stay_paused);
        }
        true
    }

    /// Force-reload the currently-playing track after a connection
    /// failover, resuming at the current playback position so a wifi→
    /// cellular switch (or any connection change) doesn't restart it from
    /// 0:00. Not gated on `last_retried_track` — the connection just
    /// changed, so a prior retry is no longer informative. Direct-play
    /// tracks that are happily buffering see a brief audio gap, but the
    /// alternative is a 15s hang the next time mpv reaches the now-dead
    /// upstream. See [`reload_current_track`] for the resume mechanics.
    pub fn force_reload_current_track(&self) -> bool {
        let (resume, idx) = {
            let mut inner = self.inner.lock();
            // Coalesce a burst of failover triggers: if an automatic reload
            // just fired, let it play out rather than stacking another.
            if inner.within_reload_cooldown() {
                log::debug!("force_reload_current_track: within reload cooldown, skipping");
                return false;
            }
            inner.last_auto_reload_at = Some(Instant::now());
            (inner.position, inner.state.queue_index)
        };
        self.reload_current_track(Some(resume), Some(idx))
    }

    /// Handle mpv idle-active. Returns `true` when the queue genuinely
    /// completed and the stopped-state teardown ran — the platform layer
    /// gates its own teardown (lock-screen clear, stopped report, prefetch
    /// cancel) on this. Returns `false` while held for recovery: a held
    /// player's mpv going idle is *expected* (the failed stream ended and
    /// the auto-advance walk ran out of playlist), and tearing the hold
    /// down here would turn a recoverable outage into a dead Stopped
    /// session — `reload_current_track` declines on Stopped, so not even a
    /// later network recovery could revive it.
    #[must_use]
    pub fn handle_idle_active(&self) -> bool {
        let mut inner = self.inner.lock();
        if inner.held_for_recovery {
            log::debug!("idle_active ignored: held for recovery");
            return false;
        }
        inner.state.status = PlaybackStatus::Stopped;
        inner.state.current_track = None;
        inner.position = 0.0;
        inner.position_base = 0.0;
        inner.load_started_at = None;
        inner.last_position_update = None;
        inner.held_for_recovery = false;
        inner.last_auto_reload_at = None;
        inner.last_recovery_reload_at = None;
        inner.reloading_pos = None;
        inner.reload_started_at = None;
        true
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

            let needs_transcode = transcode::should_transcode(
                track.codec.as_deref(),
                inner.config.playback_mode,
                inner.is_remote,
                inner.is_cellular,
            );

            if needs_transcode {
                // Session shape is `<client-id>-<unique-id>` — Plex
                // tokenises on `-` for session grouping, so any extra
                // suffix risks the server conflating concurrent sessions
                // for the same client and quietly dropping one. Live and
                // prefetch for the same track use the same session value
                // (rating-key dedupes), but they only ever overlap if
                // the user is actively playing a track they're also
                // about to prefetch — and `next_uncached_target_in_lookahead`
                // already skips anything cached, so that's a no-op.
                let session = format!("{}-{}", inner.client_identifier, track.rating_key);
                let Some(url) = transcode::build_transcode_download_url(
                    server_url,
                    token,
                    &track.rating_key,
                    &inner.client_identifier,
                    &session,
                    inner.config.transcode_bitrate,
                    None,
                ) else {
                    continue;
                };
                log::debug!(
                    "next_uncached_target: idx={idx} rk={} (transcoded prefetch)",
                    track.rating_key,
                );
                return Some((track.rating_key.clone(), url.to_string()));
            }

            let Some(part_key) = track.part_key.as_ref() else {
                continue;
            };
            let Some(url) = transcode::build_direct_play_url(server_url, part_key, token) else {
                continue;
            };
            // The constructed URL contains a live `X-Plex-Token` query
            // parameter, so only log the rating key + part key — never
            // the URL itself.
            log::debug!(
                "next_uncached_target: idx={idx} rk={} part_key={}",
                track.rating_key,
                part_key,
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

    /// Returns a `WarmTarget` for every lookahead-window track whose audio
    /// is already cached (LRU prefetch or permanent download), carrying its
    /// on-disk audio path and album-art thumb. Drives the prefetch worker's
    /// lowest-priority warming tier, which fills waveform sidecars and hero
    /// art only for tracks that are already playable offline.
    ///
    /// Re-read fresh on each warming pass, so it tracks queue advancement
    /// the same way `next_uncached_target_in_lookahead` does.
    pub fn lookahead_warm_targets(&self, include_current: bool) -> Vec<WarmTarget> {
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
            // Audio-secured gate: a persistent download wins over the LRU
            // copy. Tracks with no cached audio are skipped entirely.
            let audio_path = if let Some(path) = persistent.get(&track.rating_key) {
                path.clone()
            } else if let Some(path) = inner.cache.get(&track.rating_key) {
                path.to_path_buf()
            } else {
                continue;
            };
            out.push(WarmTarget {
                rating_key: track.rating_key.clone(),
                audio_path,
                thumb: track.thumb.clone(),
            });
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

    /// Whether the currently-playing track's source bytes have been fully
    /// pulled by mpv (i.e. the source HTTP body has EOFed and mpv is
    /// playing from its in-memory buffer the rest of the way through).
    ///
    /// Compares `demuxer-cache-time` against the Plex-DB `Track.duration`
    /// (immutable, set at sync time), NOT mpv's reported `inner.duration`.
    /// mpv's estimate grows in step with `demuxer-cache-time` while
    /// buffering a chunked Ogg stream, so checking against it always
    /// returns true. The DB duration breaks that coupling. 0.25s slack
    /// covers float jitter.
    ///
    /// Note: this is a "demuxer is at-or-near end" check, not a "file
    /// on disk is structurally complete" check. The recorder's
    /// libavformat may still hold tail packets in its internal page
    /// buffer even after the demuxer reports drained — callers needing
    /// a structurally valid file should clamp reads at the last
    /// complete Ogg page boundary (see `bounded_ogg_source` in the
    /// spectrum analyser) rather than relying on this predicate alone.
    ///
    /// Returns `false` if the queue is empty, the track has no DB
    /// duration, or the bridge doesn't expose `demuxer-cache-time`.
    pub fn current_source_fully_drained(&self) -> bool {
        let inner = self.inner.lock();
        let Some(track) = inner.state.queue.get(inner.state.queue_index) else {
            return false;
        };
        let Some(cache_time) = self.mpv.demuxer_cache_time() else {
            return false;
        };
        let position = inner.position;
        let duration = track.duration;
        if duration <= 0.0 {
            return false;
        }
        let needed = (duration - position - 0.25).max(0.0);
        cache_time >= needed
    }

    /// Raw `demuxer-cache-time` from the underlying mpv bridge, exposed
    /// for callers (the prefetch worker) that need to track changes
    /// across polls — e.g. confirming the demuxer has actually stopped
    /// pulling, not just "almost there".
    ///
    /// `None` when the bridge can't report the property (mobile bridges
    /// that haven't grown the call yet) or mpv has nothing buffered.
    pub fn demuxer_cache_time(&self) -> Option<f64> {
        self.mpv.demuxer_cache_time()
    }

    /// Approximate expected on-disk size for the currently-playing
    /// track's source body, in bytes. Drain detection compares this
    /// against the actual stream-record file size to decide whether
    /// Plex has finished sending the body.
    ///
    /// For transcoded tracks: `duration × transcode_bitrate / 8`. The
    /// Opus encoder is VBR so this is approximate (callers should use a
    /// 95% threshold). For direct-play: prefers the exact
    /// `Track.file_size_bytes` populated at sync time, falling back to
    /// `duration × Track.bitrate / 8` if the column wasn't populated.
    ///
    /// Returns `None` when the queue is empty, the track has no
    /// duration, or no usable bitrate / size hint is available.
    pub fn expected_source_bytes_for_current(&self) -> Option<u64> {
        let inner = self.inner.lock();
        let track = inner.state.queue.get(inner.state.queue_index)?;
        if track.duration <= 0.0 {
            return None;
        }
        let needs_transcode = transcode::should_transcode(
            track.codec.as_deref(),
            inner.config.playback_mode,
            inner.is_remote,
            inner.is_cellular,
        );
        if needs_transcode {
            let kbps = inner.config.transcode_bitrate.as_kbps() as f64;
            Some((track.duration * kbps * 1000.0 / 8.0) as u64)
        } else if let Some(sz) = track.file_size_bytes.filter(|s| *s > 0) {
            Some(sz as u64)
        } else {
            let kbps = track.bitrate.filter(|b| *b > 0)? as f64;
            Some((track.duration * kbps * 1000.0 / 8.0) as u64)
        }
    }

}

/// How a resume position should be realised after (re)loading a track.
/// The two seek mechanisms are kept distinct because a transcode stream
/// can't be byte-range sought.
enum ResumePlan {
    /// No resume — play from the top.
    None,
    /// Seek via an mpv `start=<secs>` per-file option. Used for local
    /// files and direct-play URLs (HTTP-Range-seekable), so mpv's reported
    /// timeline stays absolute and no position remap is needed.
    MpvSeek(f64),
    /// The resume is baked into a transcode `offset=` URL. mpv sees a
    /// fresh stream starting at 0, so the player shifts reported positions
    /// by this many seconds (`position_base`) back onto the track timeline.
    StreamOffset(f64),
}

/// Resolve a track's playback URL for a normal (from-the-top) load.
fn resolve_url(
    track: &Track,
    inner: &PlayerInner,
    persistent: &HashMap<String, PathBuf>,
) -> Option<String> {
    resolve_url_with_resume(track, inner, persistent, None).map(|(url, _)| url)
}

/// Resolve a track's playback URL, optionally resuming `resume` seconds in
/// (connection-failover reload / backward-seek of an offset stream).
/// Returns the URL and a [`ResumePlan`] telling the caller how to reach
/// the resume point. `resume` of `Some(v)` with `v <= 0` is treated as no
/// resume.
fn resolve_url_with_resume(
    track: &Track,
    inner: &PlayerInner,
    persistent: &HashMap<String, PathBuf>,
    resume: Option<f64>,
) -> Option<(String, ResumePlan)> {
    let resume = resume.filter(|p| *p > 0.0);

    if let Some(path) = persistent.get(&track.rating_key) {
        let plan = resume.map_or(ResumePlan::None, ResumePlan::MpvSeek);
        return Some((format!("file://{}", path.display()), plan));
    }
    if let Some(path) = inner.cache.get(&track.rating_key) {
        let plan = resume.map_or(ResumePlan::None, ResumePlan::MpvSeek);
        return Some((format!("file://{}", path.display()), plan));
    }

    let server_url = inner.server_url.as_ref()?;
    let token = inner.token.as_ref()?;

    if transcode::should_transcode(
        track.codec.as_deref(),
        inner.config.playback_mode,
        inner.is_remote,
        inner.is_cellular,
    ) {
        // Single-file Opus instead of HLS. Plex enforces a per-client
        // concurrent-transcode cap of ~1, and a long-lived HLS session
        // (which lasts the full real-time duration of the song) gets
        // killed the moment the prefetch worker opens a second transcode
        // session for the next track. Single-file completes in seconds —
        // mpv slurps the whole 3-5 MB file into its forward buffer at
        // server-transcode speed, the session ends, and prefetch can run
        // without competition. Session shape mirrors the prefetch path:
        // `<client-id>-<rating-key>` — Plex tokenises on `-` for session
        // grouping, so extra suffixes risk it conflating two sessions
        // for the same client.
        let session = format!("{}-{}", inner.client_identifier, track.rating_key);
        // Resume is served by the server-side `offset=` (see
        // `build_transcode_download_url`) rather than an mpv seek: a
        // transcode stream is `Accept-Ranges: none`, so an mpv `start=`
        // would force a read-through from byte 0. Sub-second offsets are
        // dropped (meaningless, and Plex's offset is integer seconds).
        let offset = resume.map(|p| p as u64).filter(|s| *s > 0);
        let url = transcode::build_transcode_download_url(
            server_url,
            token,
            &track.rating_key,
            &inner.client_identifier,
            &session,
            inner.config.transcode_bitrate,
            offset,
        )?;
        let plan = match offset {
            Some(secs) => ResumePlan::StreamOffset(secs as f64),
            None => ResumePlan::None,
        };
        Some((url.to_string(), plan))
    } else {
        let part_key = track.part_key.as_ref()?;
        let url = transcode::build_direct_play_url(server_url, part_key, token)?;
        let plan = resume.map_or(ResumePlan::None, ResumePlan::MpvSeek);
        Some((url.to_string(), plan))
    }
}

/// Build the per-file mpv `stream-record=<path>` option for a track being
/// loaded into the playlist, or `None` if recording isn't applicable.
///
/// Returns `None` for:
/// - Tracks without a configured `stream_record_dir` (feature off).
/// - URLs already pointing at a local file (no point recording a copy).
///
/// Forward slashes in the path are required because mpv's options parser
/// treats `\` as an escape character. The destination filename uses
/// `<rating_key>.<ext>` so the spectrum analyser's symphonia probe gets
/// a useful extension hint, and the file is unique per track.
fn stream_record_option_for(track: &Track, url: &str, inner: &PlayerInner) -> Option<String> {
    let dir = inner.stream_record_dir.as_ref()?;
    if url.starts_with("file://") {
        return None;
    }
    let is_transcode = transcode::should_transcode(
        track.codec.as_deref(),
        inner.config.playback_mode,
        inner.is_remote,
        inner.is_cellular,
    );

    // Transcoded sources always come back as Ogg/Opus from Plex's
    // `/audio/:/transcode/universal/start` endpoint. For direct-play,
    // try the URL extension and fall back to the codec field — either
    // is good enough for symphonia's `Hint::with_extension`.
    let ext = if is_transcode {
        "ogg".to_string()
    } else {
        // Strip the query string before grabbing the extension —
        // rsplit was returning the query (everything after `?`) and
        // the codec field was always the de-facto fallback.
        url.split('?')
            .next()
            .and_then(|p| p.rsplit('.').next())
            .filter(|e| {
                !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric())
            })
            .map(|s| s.to_ascii_lowercase())
            .or_else(|| track.codec.as_ref().map(|c| c.to_ascii_lowercase()))
            .unwrap_or_else(|| "audio".to_string())
    };

    let path = dir.join(format!("{}.{}", track.rating_key, ext));
    let path_str = path.to_string_lossy().replace('\\', "/");
    Some(format!("stream-record=\"{path_str}\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::mpv::MpvPlayer;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

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
    fn test_pos_change_to_zero_after_start_at_is_suppressed() {
        // load_queue with start_at > 0 issues `loadfile Replace` for queue[0],
        // which makes mpv fire playlist-pos-change(0) before the explicit
        // playlist_play_index lands. That transient event must not mutate
        // current_track or queue_index away from the requested start.
        let (player, _) = make_player();
        let tracks = vec![
            make_test_track("A"),
            make_test_track("B"),
            make_test_track("C"),
        ];

        player.load_queue(tracks, 2);
        assert_eq!(player.state().current_track.as_ref().unwrap().rating_key, "C");

        // Transient pos=0 event from mpv: must be ignored (not an advance).
        assert!(!player.handle_playlist_pos_change(0));
        assert_eq!(
            player.state().current_track.as_ref().unwrap().rating_key,
            "C",
            "transient pos=0 must not flip current_track"
        );
        assert_eq!(player.state().queue_index, 2);

        // Real pos=2 event arrives; gate clears, state stays consistent.
        assert!(player.handle_playlist_pos_change(2));
        assert_eq!(player.state().current_track.as_ref().unwrap().rating_key, "C");

        // Subsequent natural advance to pos=0 (e.g. user clicks back to start)
        // is now processed normally because the gate cleared.
        assert!(player.handle_playlist_pos_change(0));
        assert_eq!(player.state().current_track.as_ref().unwrap().rating_key, "A");
        assert_eq!(player.state().queue_index, 0);
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
    fn test_handle_duration_change_ignored_when_metadata_present() {
        // load_queue seeds duration from track.duration (180.0) — mpv's
        // own report is ignored to keep the seek bar stable on chunked
        // streams that don't have a Content-Length.
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        assert!((player.duration() - 180.0).abs() < 0.01);

        player.handle_duration_change(200.0);
        assert!((player.duration() - 180.0).abs() < 0.01);
    }

    #[test]
    fn test_handle_duration_change_accepted_when_no_metadata() {
        // When metadata duration is 0 (rare), mpv's report fills in.
        let (player, _) = make_player();
        let mut track = make_test_track("1");
        track.duration = 0.0;
        player.load_queue(vec![track], 0);
        assert_eq!(player.duration(), 0.0);

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

        assert!(player.handle_playlist_pos_change(2));

        let state = player.state();
        assert_eq!(state.queue_index, 2);
        assert_eq!(state.current_track.as_ref().unwrap().rating_key, "3");
        assert_eq!(player.position(), 0.0);
    }

    #[test]
    fn test_reload_pos_change_preserves_resume_position() {
        let (player, _) = make_player();
        player.update_config(PlaybackConfig {
            playback_mode: PlaybackMode::Always,
            ..PlaybackConfig::default()
        });
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_position_change(90.0);
        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        // Failover reload arms `reloading_pos` and bakes in the transcode
        // offset base (=90).
        assert!(player.force_reload_current_track());

        // mpv's insert/play/remove dance re-enters index 0 and fires a
        // pos-change. It must report "not an advance" AND must not zero the
        // resume position/base (which would snap the seek bar to 0:00).
        assert!(!player.handle_playlist_pos_change(0));

        // The base is preserved, so a fresh 0-based transcode tick maps back
        // onto the real timeline (~95s), not ~5s.
        player.handle_position_change(5.0);
        assert!(
            (player.position() - 95.0).abs() < 0.5,
            "reload must preserve the transcode offset base, got {}",
            player.position()
        );
    }

    #[test]
    fn test_reload_suppresses_intermediate_insert_shift() {
        // Reproduces the mobile failover glitch: mpv's insert-at pushes the
        // playing entry from idx 0 to idx 1, firing a pos-change at the shifted
        // index *before* play_index lands it back on 0. The intermediate event
        // must not be mistaken for an advance to track 2 (which cleared the
        // waveform and snapped the seek bar to 0:00).
        let (player, _) = make_player();
        player.update_config(PlaybackConfig {
            playback_mode: PlaybackMode::Always,
            ..PlaybackConfig::default()
        });
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        player.handle_position_change(90.0);
        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        assert!(player.force_reload_current_track());

        // Transient insert-shift event at idx+1 (= track "2"): suppressed, and
        // it must NOT switch the current track or zero the resume base.
        assert!(!player.handle_playlist_pos_change(1));
        assert_eq!(player.state().queue_index, 0);
        assert_eq!(
            player.state().current_track.as_ref().unwrap().rating_key,
            "1"
        );

        // Landing event back on the reload index: also "not an advance".
        assert!(!player.handle_playlist_pos_change(0));
        assert_eq!(player.state().queue_index, 0);

        // Base survived both events, so a fresh 0-based transcode tick maps
        // back onto the real timeline (~95s), not ~5s.
        player.handle_position_change(5.0);
        assert!(
            (player.position() - 95.0).abs() < 0.5,
            "reload must preserve the transcode offset base through the insert \
             shift, got {}",
            player.position()
        );
    }

    #[test]
    fn test_reload_settle_window_elapses_to_allow_advance() {
        // Once the settle window elapses without a landing event, a genuine
        // advance must be honoured again (the window is a backstop, not a
        // permanent gag).
        let (player, _) = make_player();
        player.update_config(PlaybackConfig {
            playback_mode: PlaybackMode::Always,
            ..PlaybackConfig::default()
        });
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        assert!(player.force_reload_current_track());

        // Force the window open long enough to expire.
        player.inner.lock().reload_started_at =
            Some(Instant::now() - RELOAD_SETTLE_WINDOW - Duration::from_millis(1));

        // A real advance to track "2" is now honoured.
        assert!(player.handle_playlist_pos_change(1));
        assert_eq!(player.state().queue_index, 1);
        assert_eq!(
            player.state().current_track.as_ref().unwrap().rating_key,
            "2"
        );
    }

    #[test]
    fn test_handle_playlist_pos_change_negative_is_ignored() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);

        assert!(!player.handle_playlist_pos_change(-1));
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

        assert!(player.handle_idle_active());

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
    fn test_derive_phase_states() {
        let now = Instant::now();
        let recent = now - Duration::from_secs(1);
        let stale = now - Duration::from_secs(STALL_THRESHOLD_SECS + 1);

        // No load yet, status=Playing → Opening (shouldn't happen in practice
        // but `derive_phase` shouldn't panic).
        assert_eq!(
            derive_phase(PlaybackStatus::Playing, None, None, now),
            Phase::Opening,
        );
        // Load just kicked off, no time-pos yet → Buffering.
        assert_eq!(
            derive_phase(PlaybackStatus::Playing, None, Some(recent), now),
            Phase::Buffering,
        );
        // Load happened, position has been arriving → Playing.
        assert_eq!(
            derive_phase(PlaybackStatus::Playing, Some(recent), Some(recent), now),
            Phase::Playing,
        );
        // Load happened, no time-pos for ages → Stalled.
        assert_eq!(
            derive_phase(PlaybackStatus::Playing, None, Some(stale), now),
            Phase::Stalled,
        );
        // Position events came in then dried up → Stalled.
        assert_eq!(
            derive_phase(PlaybackStatus::Playing, Some(stale), Some(stale), now),
            Phase::Stalled,
        );
        // Paused / Stopped passthrough.
        assert_eq!(
            derive_phase(PlaybackStatus::Paused, Some(stale), Some(stale), now),
            Phase::Paused,
        );
        assert_eq!(
            derive_phase(PlaybackStatus::Stopped, None, None, now),
            Phase::Stopped,
        );
    }

    #[test]
    fn test_load_queue_seeds_phase_timestamps() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);

        // Fresh load → Buffering (load_started_at set, no position update yet).
        let snap = player.debug_snapshot();
        assert_eq!(snap.phase, Phase::Buffering);
        assert!(snap.seconds_since_load.is_some());
        assert!(snap.seconds_since_position_update.is_none());

        // First time-pos lands → Playing.
        player.handle_position_change(0.5);
        let snap = player.debug_snapshot();
        assert_eq!(snap.phase, Phase::Playing);
        assert_eq!(snap.seconds_since_position_update, Some(0));
    }

    #[test]
    fn test_resume_resets_progress_timer() {
        // Long pause shouldn't make resumed playback look stalled.
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_position_change(10.0);

        // Backdate the position timestamp to simulate a pause longer than
        // the stall threshold.
        {
            let mut inner = player.inner.lock();
            inner.last_position_update =
                Some(Instant::now() - Duration::from_secs(STALL_THRESHOLD_SECS + 5));
        }
        player.handle_pause_change(true);
        player.handle_pause_change(false);

        assert_eq!(player.debug_snapshot().phase, Phase::Playing);
        assert!(!player.is_stalled());
    }

    #[test]
    fn test_media_position_snapshot_detects_mid_track_stall() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        // A fresh position tick: playing, progressing, not stalled.
        player.handle_position_change(40.0);
        let threshold = Duration::from_millis(2000);
        let snap = player.media_position_snapshot(threshold);
        assert!(snap.is_playing);
        assert!(!snap.progress_stalled);
        assert!((snap.position - 40.0).abs() < 0.01);

        // Backdate the last tick past the threshold: audio has stalled, the OS
        // scrubber must be frozen at the true position (still 40s, not
        // extrapolated forward).
        player.inner.lock().last_position_update =
            Some(Instant::now() - threshold - Duration::from_millis(500));
        let snap = player.media_position_snapshot(threshold);
        assert!(snap.is_playing);
        assert!(snap.progress_stalled);
        assert!((snap.position - 40.0).abs() < 0.01);

        // Paused is never a "stall" — the pause push owns the OS surface.
        player.handle_pause_change(true);
        let snap = player.media_position_snapshot(threshold);
        assert!(!snap.is_playing);
        assert!(!snap.progress_stalled);
    }

    #[test]
    fn test_file_ended_error_records_redacted_message() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);

        let leaky = "GET https://srv:32400/x?X-Plex-Token=SECRET failed";
        player.handle_file_ended(FileEndReason::Error(leaky.into()));

        let err = player.debug_snapshot().last_load_error.unwrap();
        assert!(!err.contains("SECRET"));
    }

    #[test]
    fn test_handle_file_ended_error_resumes_then_holds() {
        let (player, _) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2")],
            0,
        );

        // First error: resume-at-position reload — stays on track 0.
        let out = player.handle_file_ended(FileEndReason::Error("test".into()));
        assert!(matches!(out, RecoverOutcome::Reloading(_)), "got {out:?}");
        assert_eq!(player.state().queue_index, 0);

        // Second error on the same track: hold at position (never skip or reset
        // to 0:00), so playback stays on track 0, paused, awaiting a play tap.
        let out = player.handle_file_ended(FileEndReason::Error("test".into()));
        assert!(matches!(out, RecoverOutcome::Held(_)), "got {out:?}");
        assert_eq!(player.state().queue_index, 0);
        assert_eq!(player.state().status, PlaybackStatus::Paused);
    }

    #[test]
    fn test_force_reload_coalesces_within_cooldown() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_position_change(30.0);
        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        // First failover reload fires.
        assert!(player.force_reload_current_track());
        // A second trigger arriving immediately (network-path flap + stall
        // watchdog + prefetch all firing for one hiccup) is coalesced by the
        // reload cooldown rather than stacked into another reload.
        assert!(
            !player.force_reload_current_track(),
            "second reload within cooldown must be suppressed"
        );
    }

    /// Drive a player into the held-for-recovery state: two consecutive
    /// load errors on the same track exhaust the retry and hold at position.
    fn hold_player_at(player: &AudioPlayer, pos: f64) {
        player.handle_position_change(pos);
        player.handle_file_ended(FileEndReason::Error("test".into()));
        let out = player.handle_file_ended(FileEndReason::Error("test".into()));
        assert!(matches!(out, RecoverOutcome::Held(_)), "got {out:?}");
        assert_eq!(player.state().status, PlaybackStatus::Paused);
    }

    #[test]
    fn test_recover_interrupted_playback_resumes_held() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        hold_player_at(&player, 30.0);
        let calls_before = mpv.call_count();

        // The network came back (recovered edge / healthy watchdog verdict):
        // the hold must exit into a resume-at-position reload even though the
        // hold itself stamped the auto-reload cooldown moments ago.
        assert!(player.recover_interrupted_playback());
        assert_eq!(player.state().status, PlaybackStatus::Playing);
        assert!(!player.needs_connection_recovery());

        // The reload is the insert/play/remove dance on the held index.
        let calls = mpv.calls()[calls_before..].to_vec();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, MockCall::LoadFileAt { index: 0, .. })),
            "expected a reload of the held entry, got {calls:?}"
        );
    }

    #[test]
    fn test_recover_interrupted_playback_respects_user_pause() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        hold_player_at(&player, 30.0);

        // A pause while held is swallowed by the status gate (status is
        // already Paused), but the *intent* must be recorded — recovery
        // must never blast audio at a user who asked for silence.
        player.pause();
        assert!(!player.needs_connection_recovery());

        let calls_before = mpv.call_count();
        assert!(!player.recover_interrupted_playback());
        assert_eq!(mpv.call_count(), calls_before, "no mpv command expected");
        assert_eq!(player.state().status, PlaybackStatus::Paused);

        // An explicit resume is the user's own retry: it re-attempts the
        // held load and clears the pause intent.
        player.resume();
        assert_eq!(player.state().status, PlaybackStatus::Playing);
    }

    #[test]
    fn test_recover_interrupted_playback_reloads_stalled_stream() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_position_change(42.0);

        // Healthy stream: nothing to recover.
        assert!(!player.needs_connection_recovery());
        assert!(!player.recover_interrupted_playback());

        // A dead socket after a silent network flip: status stays Playing
        // but position ticks stop (mpv never errors). Recovery must kick
        // the stream with a resume-at-position reload.
        player.inner.lock().last_position_update =
            Some(Instant::now() - Duration::from_secs(STALL_THRESHOLD_SECS + 1));
        assert!(player.needs_connection_recovery());
        let calls_before = mpv.call_count();
        assert!(player.recover_interrupted_playback());
        let calls = mpv.calls()[calls_before..].to_vec();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, MockCall::LoadFileAt { index: 0, .. })),
            "expected a reload of the stalled entry, got {calls:?}"
        );
    }

    #[test]
    fn test_recover_interrupted_playback_coalesces_within_cooldown() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_position_change(42.0);
        player.inner.lock().last_position_update =
            Some(Instant::now() - Duration::from_secs(STALL_THRESHOLD_SECS + 1));

        assert!(player.recover_interrupted_playback());
        // Still stalled (no position tick arrived) — a second racing trigger
        // (path event + watchdog) must coalesce, not stack reloads.
        player.inner.lock().last_position_update =
            Some(Instant::now() - Duration::from_secs(STALL_THRESHOLD_SECS + 1));
        assert!(!player.recover_interrupted_playback());
    }

    #[test]
    fn test_user_pause_intent_survives_status_gate_and_clears_on_play() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_position_change(10.0);

        player.pause();
        assert!(player.inner.lock().user_paused);
        player.resume();
        assert!(!player.inner.lock().user_paused);

        player.toggle_play_pause(); // Playing -> Paused
        assert!(player.inner.lock().user_paused);
        player.toggle_play_pause(); // Paused -> Playing
        assert!(!player.inner.lock().user_paused);

        // A fresh queue load supersedes any lingering pause intent.
        player.pause();
        player.load_queue(vec![make_test_track("2")], 0);
        assert!(!player.inner.lock().user_paused);
    }

    /// The `SetPause` values sent to mpv, in order.
    fn pause_calls(mpv: &MockMpv) -> Vec<bool> {
        mpv.calls()
            .iter()
            .filter_map(|c| match c {
                MockCall::SetPause(v) => Some(*v),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn test_hold_entry_pins_mpv_paused() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        hold_player_at(&player, 30.0);

        // Entering the hold must park mpv paused — with keep-open=no it
        // auto-advances past the failed entry, and an unpinned walk plays
        // whatever entry loads next, audibly, under a Paused status.
        assert_eq!(pause_calls(&mpv).last(), Some(&true));
    }

    #[test]
    fn test_pos_change_suppressed_while_held() {
        let (player, _) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
            0,
        );
        hold_player_at(&player, 30.0);

        // mpv's auto-advance walk fires pos-changes for the entries it
        // moves through. None are real advances: the hold owns the queue
        // position, and processing them used to clear the hold and cascade
        // the pointer through the whole queue.
        assert!(!player.handle_playlist_pos_change(1));
        assert!(!player.handle_playlist_pos_change(2));
        let state = player.state();
        assert_eq!(state.queue_index, 0);
        assert_eq!(
            state.current_track.as_ref().map(|t| t.rating_key.as_str()),
            Some("1")
        );
        assert!(player.inner.lock().held_for_recovery);
    }

    #[test]
    fn test_idle_active_preserved_while_held() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        hold_player_at(&player, 30.0);

        // The walk exhausting the playlist idles mpv. For a held player
        // that is expected — tearing down to Stopped would make the hold
        // unrecoverable (reloads decline on Stopped).
        assert!(!player.handle_idle_active());
        let state = player.state();
        assert_eq!(state.status, PlaybackStatus::Paused);
        assert!(state.current_track.is_some());
        assert!(player.inner.lock().held_for_recovery);

        // The preserved hold is still recoverable.
        assert!(player.recover_interrupted_playback());
        assert_eq!(player.state().status, PlaybackStatus::Playing);
    }

    #[test]
    fn test_resume_exits_hold_with_unpause() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        hold_player_at(&player, 30.0);

        player.resume();
        // The exit must lift the hold's pause pin, or the reloaded track
        // sits silent under a Playing status.
        assert_eq!(pause_calls(&mpv).last(), Some(&false));
        assert_eq!(player.state().status, PlaybackStatus::Playing);
    }

    #[test]
    fn test_seek_while_held_and_user_paused_stays_paused() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        hold_player_at(&player, 30.0);
        player.pause();

        // A paused user dragging the scrubber reloads the held track at
        // the target, but silent — their pause intent survives the exit.
        player.seek(50.0);
        assert!(!player.inner.lock().held_for_recovery);
        assert_eq!(player.state().status, PlaybackStatus::Paused);
        assert_eq!(pause_calls(&mpv).last(), Some(&true));
    }

    #[test]
    fn test_next_out_of_hold_plays() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        hold_player_at(&player, 30.0);

        player.next();
        let state = player.state();
        assert_eq!(state.queue_index, 1);
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert!(!player.inner.lock().held_for_recovery);
        // Skip = play intent: the hold's pause pin must lift.
        assert_eq!(pause_calls(&mpv).last(), Some(&false));

        // The skip's own confirmation pos-change is processed normally
        // (the hold was released before the command).
        assert!(player.handle_playlist_pos_change(1));
    }

    #[test]
    fn test_next_out_of_hold_respects_user_pause() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        hold_player_at(&player, 30.0);
        player.pause();

        player.next();
        let state = player.state();
        assert_eq!(state.queue_index, 1);
        assert_eq!(state.status, PlaybackStatus::Paused);
        // No unpause: the user asked for silence; the next track loads
        // paused under mpv's sticky pause.
        assert_eq!(pause_calls(&mpv).last(), Some(&true));
    }

    #[test]
    fn test_previous_while_held_reloads_instead_of_dead_seek() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        hold_player_at(&player, 30.0);
        let calls_before = mpv.call_count();

        // A held track has no live mpv stream: previous()'s restart-current
        // branch must reload from the top, not issue a silent no-op seek.
        player.previous();
        let calls = mpv.calls()[calls_before..].to_vec();
        assert!(
            calls
                .iter()
                .any(|c| matches!(c, MockCall::LoadFileAt { index: 0, .. })),
            "expected a restart reload, got {calls:?}"
        );
        assert!(!calls.iter().any(|c| matches!(c, MockCall::Seek(_))));
        assert_eq!(player.state().status, PlaybackStatus::Playing);
    }

    #[test]
    fn test_load_queue_clears_hold() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        hold_player_at(&player, 30.0);

        // A fresh queue moots the hold; without the release, the new
        // queue's pos-change events would be suppressed as the walk.
        player.load_queue(vec![make_test_track("3")], 0);
        assert!(!player.inner.lock().held_for_recovery);
        assert!(player.handle_playlist_pos_change(0));
    }

    #[test]
    fn test_jump_to_index_is_play_intent() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        player.handle_position_change(10.0);
        player.pause();

        // Tapping a track is an explicit "play this": it supersedes the
        // pause intent and lifts mpv's sticky pause — without the explicit
        // unpause, the selected track sits silent under a Playing status.
        player.jump_to_index(1);
        let state = player.state();
        assert_eq!(state.queue_index, 1);
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert!(!player.inner.lock().user_paused);
        assert_eq!(pause_calls(&mpv).last(), Some(&false));
    }

    #[test]
    fn test_jump_out_of_hold_plays_even_when_user_paused() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        hold_player_at(&player, 30.0);
        player.pause();

        // Unlike next/previous (which preserve the pause), an explicit
        // track selection during an outage means "play this one now".
        player.jump_to_index(1);
        let state = player.state();
        assert_eq!(state.queue_index, 1);
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert!(!player.inner.lock().held_for_recovery);
        assert_eq!(pause_calls(&mpv).last(), Some(&false));

        // The confirmation pos-change is processed normally.
        assert!(player.handle_playlist_pos_change(1));
    }

    #[test]
    fn test_natural_advance_records_transition_at_final_position() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        player.handle_position_change(175.0);

        // The advance mutates state to track 2 at 0:00 — the snapshot must
        // preserve track 1 at the position it actually ended.
        assert!(player.handle_playlist_pos_change(1));
        let (prev, pos, dur) = player.take_pending_transition().unwrap();
        assert_eq!(prev.rating_key, "1");
        assert_eq!(pos, 175.0);
        assert_eq!(dur, 180.0);

        // Consumed — a second take yields nothing.
        assert!(player.take_pending_transition().is_none());
    }

    #[test]
    fn test_skip_confirmation_keeps_preskip_position() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        player.handle_position_change(42.0);

        // The skip records the transition synchronously; mpv's confirmation
        // pos-change (same rating key, position already zeroed) must not
        // clobber the snapshot with 0:00.
        player.next();
        assert!(player.handle_playlist_pos_change(1));
        let (prev, pos, _) = player.take_pending_transition().unwrap();
        assert_eq!(prev.rating_key, "1");
        assert_eq!(pos, 42.0);
    }

    #[test]
    fn test_previous_records_transition() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        assert!(player.handle_playlist_pos_change(1));
        let _ = player.take_pending_transition();
        player.handle_position_change(2.0);

        // Under the 3s restart threshold, previous() goes back a track.
        player.previous();
        let (prev, pos, _) = player.take_pending_transition().unwrap();
        assert_eq!(prev.rating_key, "2");
        assert_eq!(pos, 2.0);
    }

    #[test]
    fn test_jump_records_transition() {
        let (player, _) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2"), make_test_track("3")],
            0,
        );
        player.handle_position_change(60.0);

        player.jump_to_index(2);
        let (prev, pos, _) = player.take_pending_transition().unwrap();
        assert_eq!(prev.rating_key, "1");
        assert_eq!(pos, 60.0);
    }

    #[test]
    fn test_skip_off_queue_end_records_transition() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_position_change(171.0);

        // No pos-change fires on this path (mpv just goes idle) — the idle
        // callback consumes the snapshot to close the track at 95%, where
        // it deserves its scrobble.
        player.next();
        assert_eq!(player.state().status, PlaybackStatus::Stopped);
        let (prev, pos, _) = player.take_pending_transition().unwrap();
        assert_eq!(prev.rating_key, "1");
        assert_eq!(pos, 171.0);
    }

    #[test]
    fn test_load_queue_and_stop_clear_pending_transition() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        player.handle_position_change(42.0);
        player.next();

        // A fresh queue's close-out is reported by the caller with the
        // player still in its pre-load state — a leftover snapshot must not
        // fire on the new queue's first pos-change.
        player.load_queue(vec![make_test_track("3")], 0);
        assert!(player.take_pending_transition().is_none());

        // stop() likewise discards any unconsumed snapshot outright.
        player.handle_position_change(42.0);
        player.next();
        player.stop();
        assert!(player.take_pending_transition().is_none());
    }

    #[test]
    fn test_queue_reload_pos_change_records_nothing() {
        let (player, _) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);

        // The Replace load's pos-change re-confirms the track load_queue
        // already installed; play_tracks reported that start itself.
        assert!(player.handle_playlist_pos_change(0));
        assert!(player.take_pending_transition().is_none());
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

    #[test]
    fn test_rewrite_stale_playlist_urls_replaces_non_cached() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![
                make_test_track("1"),
                make_test_track("2"),
                make_test_track("3"),
            ],
            0,
        );

        mpv.calls.lock().clear();

        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        player.rewrite_stale_playlist_urls();

        let calls = mpv.calls();
        let removes: Vec<_> = calls
            .iter()
            .filter(|c| matches!(c, MockCall::PlaylistRemove(_)))
            .collect();
        let inserts: Vec<_> = calls
            .iter()
            .filter(|c| matches!(c, MockCall::LoadFileAt { .. }))
            .collect();

        // Tracks 1 and 2 (indices 1, 2) should be rewritten; track 0 (current) skipped
        assert_eq!(removes.len(), 2);
        assert_eq!(inserts.len(), 2);

        // Verify new URLs contain the new server
        for call in &inserts {
            if let MockCall::LoadFileAt { url, .. } = call {
                assert!(url.contains("new.server:32400"));
                assert!(url.contains("new-token"));
            }
        }
    }

    #[test]
    fn test_rewrite_skips_cached_and_current() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![
                make_test_track("1"),
                make_test_track("2"),
                make_test_track("3"),
            ],
            0,
        );

        // Cache track "2" in LRU
        player.with_cache(|cache| {
            cache.insert("2".into(), PathBuf::from("/tmp/cached_2.flac"), 1000);
        });

        mpv.calls.lock().clear();

        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        player.rewrite_stale_playlist_urls();

        let calls = mpv.calls();
        let removes: Vec<_> = calls
            .iter()
            .filter(|c| matches!(c, MockCall::PlaylistRemove(_)))
            .collect();

        // Only track "3" (index 2) should be rewritten; "1" is current, "2" is cached
        assert_eq!(removes.len(), 1);
        if let MockCall::PlaylistRemove(idx) = removes[0] {
            assert_eq!(*idx, 2);
        }
    }

    #[test]
    fn test_rewrite_skips_persistent_downloads() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![
                make_test_track("1"),
                make_test_track("2"),
                make_test_track("3"),
            ],
            0,
        );

        player.register_persistent_download("2".into(), PathBuf::from("/downloads/2.flac"));

        mpv.calls.lock().clear();

        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        player.rewrite_stale_playlist_urls();

        let calls = mpv.calls();
        let removes: Vec<_> = calls
            .iter()
            .filter(|c| matches!(c, MockCall::PlaylistRemove(_)))
            .collect();

        // Only track "3" (index 2) rewritten; "1" is current, "2" has persistent download
        assert_eq!(removes.len(), 1);
    }

    #[test]
    fn test_lookahead_warm_targets_only_includes_cached_audio() {
        let (player, _mpv) = make_player();
        let mut t1 = make_test_track("1");
        t1.thumb = Some("/art/1".into());
        let mut t2 = make_test_track("2");
        t2.thumb = Some("/art/2".into());
        let t3 = make_test_track("3"); // thumb stays None
        player.load_queue(vec![t1, t2, t3], 0);

        // "2" cached in the LRU, "3" as a permanent download. "1" (the
        // current track) has no cached audio.
        player.with_cache(|cache| {
            cache.insert("2".into(), PathBuf::from("/tmp/2.flac"), 1000);
        });
        player.register_persistent_download("3".into(), PathBuf::from("/downloads/3.flac"));

        // include_current = true, yet "1" is excluded because its audio
        // isn't secured — we never warm extras for an unplayable track.
        let targets = player.lookahead_warm_targets(true);
        let keys: Vec<&str> = targets.iter().map(|t| t.rating_key.as_str()).collect();
        assert_eq!(keys, vec!["2", "3"]);

        let warm2 = targets.iter().find(|t| t.rating_key == "2").unwrap();
        assert_eq!(warm2.thumb.as_deref(), Some("/art/2"));
        assert_eq!(warm2.audio_path, PathBuf::from("/tmp/2.flac"));

        let warm3 = targets.iter().find(|t| t.rating_key == "3").unwrap();
        assert_eq!(warm3.thumb, None);
        assert_eq!(warm3.audio_path, PathBuf::from("/downloads/3.flac"));
    }

    #[test]
    fn test_force_reload_current_replaces_active_entry() {
        let (player, mpv) = make_player();
        player.load_queue(
            vec![make_test_track("1"), make_test_track("2")],
            0,
        );
        mpv.calls.lock().clear();

        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        let reloaded = player.force_reload_current_track();
        assert!(reloaded);

        let calls = mpv.calls();
        // Insert fresh URL at idx 0, play it, then remove the stale (now at idx 1).
        assert!(calls
            .iter()
            .any(|c| matches!(c, MockCall::LoadFileAt { index: 0, url, .. } if url.contains("new.server:32400") && url.contains("new-token"))));
        assert!(calls
            .iter()
            .any(|c| matches!(c, MockCall::PlaylistPlayIndex(0))));
        assert!(calls
            .iter()
            .any(|c| matches!(c, MockCall::PlaylistRemove(1))));
    }

    #[test]
    fn test_force_reload_current_skips_cached() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.with_cache(|cache| {
            cache.insert("1".into(), PathBuf::from("/tmp/cached_1.flac"), 1000);
        });
        mpv.calls.lock().clear();

        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        let reloaded = player.force_reload_current_track();

        assert!(!reloaded);
        assert!(mpv.calls().is_empty());
    }

    #[test]
    fn test_force_reload_current_skips_persistent_download() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1")], 0);
        player.register_persistent_download("1".into(), PathBuf::from("/downloads/1.flac"));
        mpv.calls.lock().clear();

        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        let reloaded = player.force_reload_current_track();

        assert!(!reloaded);
        assert!(mpv.calls().is_empty());
    }

    #[test]
    fn test_force_reload_current_skips_when_stopped() {
        let (player, mpv) = make_player();
        // No load_queue — status stays Stopped.
        mpv.calls.lock().clear();

        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        let reloaded = player.force_reload_current_track();

        assert!(!reloaded);
        assert!(mpv.calls().is_empty());
    }

    #[test]
    fn test_force_reload_resumes_direct_play_with_start_option() {
        let (player, mpv) = make_player();
        player.load_queue(vec![make_test_track("1"), make_test_track("2")], 0);
        // ~60s of playback elapsed before the connection changed.
        player.handle_position_change(60.0);
        mpv.calls.lock().clear();

        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        assert!(player.force_reload_current_track());

        // Direct-play (default mode is Never) resumes via an mpv `start=`
        // seek: the URL stays a plain part URL, the option carries the seek.
        let (url, options) = mpv
            .calls()
            .into_iter()
            .find_map(|c| match c {
                MockCall::LoadFileAt {
                    index: 0,
                    url,
                    options,
                } => Some((url, options)),
                _ => None,
            })
            .expect("expected a LoadFileAt at index 0");
        assert!(
            !url.contains("offset="),
            "direct-play must not use a transcode offset"
        );
        let opts = options.expect("expected a start= option for the resume");
        assert!(opts.contains("start=60"), "expected start=60.x, got {opts}");
    }

    #[test]
    fn test_force_reload_resumes_transcode_with_offset() {
        let (player, mpv) = make_player();
        player.update_config(PlaybackConfig {
            playback_mode: PlaybackMode::Always,
            ..PlaybackConfig::default()
        });
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_position_change(75.0);
        mpv.calls.lock().clear();

        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        assert!(player.force_reload_current_track());

        let (url, options) = mpv
            .calls()
            .into_iter()
            .find_map(|c| match c {
                MockCall::LoadFileAt {
                    index: 0,
                    url,
                    options,
                } => Some((url, options)),
                _ => None,
            })
            .expect("expected a LoadFileAt at index 0");
        // Transcode resume bakes the offset into the URL (server-side seek)
        // with its companion params — and carries no mpv start= option.
        assert!(url.contains("offset=75"), "expected offset=75 in url, got {url}");
        assert!(
            url.contains("mediaBufferSize=1024"),
            "expected offset companions, got {url}"
        );
        assert!(
            options.is_none(),
            "transcode resume must not use start=, got {options:?}"
        );

        // `position_base` now remaps mpv's 0-based stream: a fresh time-pos
        // of 5s reads as 80s on the track timeline.
        player.handle_position_change(5.0);
        assert!(
            (player.snapshot().position - 80.0).abs() < 0.01,
            "position should map through the offset base, got {}",
            player.snapshot().position
        );
    }

    #[test]
    fn test_transcode_resume_failure_holds_at_position() {
        let (player, mpv) = make_player();
        player.update_config(PlaybackConfig {
            playback_mode: PlaybackMode::Always,
            ..PlaybackConfig::default()
        });
        player.load_queue(vec![make_test_track("1")], 0);
        player.handle_position_change(90.0);
        player.update_server_connection(
            Url::parse("http://new.server:32400").unwrap(),
            "new-token".into(),
            true,
        );
        assert!(player.force_reload_current_track()); // offset resume, base = 90
        mpv.calls.lock().clear();

        // The offset transcode start is refused (e.g. HTTP 400) → mpv errors.
        // Recovery must NOT reset to 0:00 or skip — it holds the track at its
        // position so a play tap can re-attempt. (The immediate retry lands
        // inside the reload cooldown, so it holds rather than thrashing.)
        let out = player.handle_file_ended(FileEndReason::Error("HTTP 400 Bad Request".into()));
        assert!(matches!(out, RecoverOutcome::Held(_)), "got {out:?}");
        assert_eq!(player.state().queue_index, 0, "must not skip the track");
        assert_eq!(player.state().status, PlaybackStatus::Paused);
        assert!(
            (player.snapshot().position - 90.0).abs() < 0.5,
            "held position must stay at ~90s, not reset to 0:00"
        );

        // A play tap re-attempts a resume-at-position (offset) load, never a
        // restart from the top.
        mpv.calls.lock().clear();
        player.resume();
        let url = mpv
            .calls()
            .into_iter()
            .find_map(|c| match c {
                MockCall::LoadFileAt { index: 0, url, .. } => Some(url),
                _ => None,
            })
            .expect("expected a re-attempt LoadFileAt at index 0");
        assert!(
            url.contains("offset="),
            "re-attempt must resume at position, got {url}"
        );
    }
}
