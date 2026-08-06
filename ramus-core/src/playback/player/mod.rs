//! AudioPlayer: queue management, mpv integration, equalizer, download
//! cache. Owns the mpv handle (via `MpvPlayer` trait) and manages the
//! playback queue, track URL resolution, LRU download cache, and 10-band
//! parametric equalizer filter strings.
//!
//! The shared state (`AudioPlayer`, `PlayerInner`) lives here; the
//! behaviour is split across submodules by subsystem, each contributing
//! its own `impl AudioPlayer` block. Submodules are children of this one,
//! so they reach `PlayerInner`'s private fields directly.

mod adaptive;
mod cache;
mod diagnostics;
mod eq;
mod events;
mod queue;
mod recovery;
mod resolve;
mod starvation;
mod transport;

pub use adaptive::*;
pub use cache::*;
pub use diagnostics::*;
pub use eq::*;
pub use recovery::*;
pub use resolve::*;
pub use starvation::*;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Mutex, RwLock};
use url::Url;

use crate::models::{PlaybackConfig, PlaybackStatus, PlayerState, Track, TranscodeBitrate};
use crate::playback::download_cache::DownloadCache;
use crate::playback::mpv::MpvPlayer;

// The test module is a child of this one and reaches the whole player
// through `use super::*`. These three names are used only by tests here,
// so they are pulled in under `cfg(test)` rather than left as unused
// imports in a normal build.
#[cfg(test)]
pub(crate) use crate::models::PlaybackMode;
#[cfg(test)]
pub(crate) use crate::playback::mpv::{FileEndReason, LoadMode};

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
    /// Whether the active Plex connection is non-local.
    ///
    /// **Diagnostic only — feeds no policy.** It was the trigger for the
    /// retired `Remote` / `RemoteOrCellular` playback modes, where it stood
    /// in for "the link probably can't sustain lossless". That condition is
    /// now measured directly (see `is_starving`), which is both more accurate
    /// and covers the cases locality misses — a slow LAN, or a fast remote
    /// server. Kept for the debug panel and logs; don't wire it back into
    /// `should_transcode`.
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
    /// Rebuffer episodes on the current stream, feeding [`is_starving`].
    /// Cleared by [`PlayerInner::begin_load`] and by every other event that
    /// produces silence we mustn't blame on the network (seek, resume,
    /// current-track reload).
    starvation: StarvationTracker,
    /// Bitrate the adaptive layer has forced this session, if any. Layered
    /// on top of `should_transcode` rather than folded into it, so the
    /// baseline policy stays a pure function of stable inputs and this stays
    /// one inspectable value with one place to clear.
    ///
    /// **Never auto-restored.** If the degrade succeeds, the starvation it
    /// was diagnosed from stops — restoring on that basis would starve,
    /// degrade, restore, and flap, at the cost of an audible reload each way.
    /// It clears only on a positive external event: a real network path flip,
    /// a connection change or recovery, a settings change, or app restart
    /// (it is deliberately never persisted).
    bandwidth_degrade: Option<TranscodeBitrate>,
    /// Wall-clock of the last adaptive step, pacing them at
    /// [`DEGRADE_COOLDOWN`].
    last_degrade_at: Option<Instant>,
}

impl PlayerInner {
    /// True if an automatic reload fired within `RELOAD_COOLDOWN`. Used to
    /// coalesce a burst of failover/recovery triggers (network-path monitor,
    /// stall watchdog, prefetch) into a single reload per hiccup.
    fn within_reload_cooldown(&self) -> bool {
        self.last_auto_reload_at
            .is_some_and(|t| t.elapsed() < RELOAD_COOLDOWN)
    }

    /// Stamp the start of a fresh track load: reset the timing anchors
    /// `derive_phase` reads, drop the previous load's error, and clear the
    /// starvation history.
    ///
    /// Every site that begins playing a *different* stream must go through
    /// here. Carrying rebuffer episodes across a track boundary would let a
    /// track that starved condemn the one after it, which may well be playing
    /// from cache.
    fn begin_load(&mut self) {
        self.load_started_at = Some(Instant::now());
        self.last_position_update = None;
        self.last_load_error = None;
        self.starvation.clear();
    }

    /// Whether the current stream is starving — the link is delivering, but
    /// too slowly to keep the demuxer cache fed. See [`is_starving`].
    fn starvation_verdict(&self, now: Instant) -> bool {
        if self.state.status != PlaybackStatus::Playing {
            return false;
        }
        let Some(load) = self.load_started_at else {
            return false;
        };
        let in_progress_gap = self
            .last_position_update
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or_default();
        is_starving(
            self.starvation.episodes(),
            in_progress_gap,
            now.saturating_duration_since(load),
            now,
        )
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
                starvation: StarvationTracker::default(),
                bandwidth_degrade: None,
                last_degrade_at: None,
            }),
            persistent_cache: RwLock::new(HashMap::new()),
        }
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

    /// Whether the current connection is non-local (relayed through plex.tv
    /// or a public IP). **Diagnostic only** — see the field's note.
    pub fn is_remote(&self) -> bool {
        self.inner.lock().is_remote
    }

    /// Update only the cellular flag. Driven by the platform NetworkMonitor
    /// (NWPathMonitor on iOS, ConnectivityManager on Android). Desktop
    /// never calls this, so `is_cellular` stays `false` on those platforms.
    /// Returns whether the flag actually changed, so the caller can re-sweep
    /// already-resolved queue entries under the new policy only on a real
    /// transition.
    pub fn set_cellular(&self, is_cellular: bool) -> bool {
        let mut inner = self.inner.lock();
        let changed = inner.is_cellular != is_cellular;
        inner.is_cellular = is_cellular;
        changed
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
        // The user has just restated the policy directly, which supersedes
        // anything the adaptive layer inferred — start measuring afresh
        // against what they asked for.
        if config.playback_mode != inner.config.playback_mode
            || config.transcode_bitrate != inner.config.transcode_bitrate
        {
            inner.bandwidth_degrade = None;
            inner.last_degrade_at = None;
            inner.starvation.clear();
        }
        inner.config = config;
    }
}

#[cfg(test)]
mod tests;
