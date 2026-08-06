//! The mpv callback surface. Every handler the platform event loop calls
//! lands here and delegates into the subsystem it drives.

use std::time::{Duration, Instant};

use crate::models::PlaybackStatus;
use crate::playback::mpv::FileEndReason;
use crate::util::redact_urls;

use super::diagnostics::BUFFERING_HINT_SECS;
use super::recovery::{RecoverOutcome, RELOAD_SETTLE_WINDOW};
use super::AudioPlayer;

impl AudioPlayer {
    /// Handle mpv position change (called by event loop, ~30fps).
    pub fn handle_position_change(&self, pos: f64) {
        let mut inner = self.inner.lock();
        // `position_base` is non-zero while a transcode `offset=` resume
        // stream plays (mpv reports 0-based; the real position is shifted
        // by the resume point).
        inner.position = pos + inner.position_base;
        let now = Instant::now();
        // This tick closes whatever silence preceded it. A gap long enough to
        // mean the demuxer cache ran dry is a rebuffer episode — and the fact
        // that a tick arrived at all proves the source is still delivering,
        // which is what separates a slow link from a dead one.
        if let Some(prev) = inner.last_position_update {
            let gap = now.saturating_duration_since(prev);
            if gap >= Duration::from_secs(BUFFERING_HINT_SECS) {
                inner.starvation.record(now, gap);
            }
        }
        inner.last_position_update = Some(now);
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
        // Playing a cached track is a use — bump it to most-recently-used
        // so the LRU eviction order reflects listening, not just download
        // order (a replayed album shouldn't be first out the door).
        let rk = inner.state.queue[pos].rating_key.clone();
        inner.cache.touch(&rk);
        inner.position = 0.0;
        inner.position_base = 0.0;
        // Reseed duration from metadata on every gapless advance — see
        // load_queue's note. Stable across UI ticks regardless of mpv's
        // streamed-source duration estimation.
        inner.duration = inner.state.queue[pos].duration;
        inner.begin_load();
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
            // Episodes from before the pause are stale evidence by the time
            // playback resumes, and mpv refills its cache on the way back —
            // judge the resumed stream on its own behaviour.
            inner.starvation.clear();
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
}
