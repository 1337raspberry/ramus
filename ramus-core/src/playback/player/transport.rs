//! Transport commands (play/pause/seek/volume/stop) and the state readouts
//! the frontend polls.

use std::time::Instant;

use crate::models::{PlaybackStatus, PlayerState, Track};

use super::{AudioPlayer, AudioPlayerState};

impl AudioPlayer {
    /// Toggle between playing and paused.
    pub fn toggle_play_pause(&self) {
        // A restored queue reads as Paused but has no mpv playlist behind it,
        // so unpausing would be silent. Load it now, resuming where the
        // previous session left off.
        if let Some((idx, pos)) = self.pending_resume_target() {
            if self.ensure_materialized_at(idx, Some(pos)) {
                return;
            }
        }
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
        // See `toggle_play_pause`: a restored queue needs loading before an
        // unpause means anything.
        if let Some((idx, pos)) = self.pending_resume_target() {
            if self.ensure_materialized_at(idx, Some(pos)) {
                return;
            }
        }
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
        // Scrubbing a restored track is itself the play intent: load the
        // queue with the drag target as the resume point rather than issuing
        // a seek against an idle mpv.
        if self.pending_resume_target().is_some() {
            let (idx, clamped) = {
                let inner = self.inner.lock();
                (
                    inner.state.queue_index,
                    position.max(0.0).min((inner.duration - 0.5).max(0.0)),
                )
            };
            if self.ensure_materialized_at(idx, Some(clamped)) {
                return;
            }
        }
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
        // A seek makes mpv refill from a new point; the silence that follows
        // is the user's doing, not the link's.
        inner.starvation.clear();
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
        // The queue is gone, so there is nothing left to materialise. Leaving
        // this armed would make the next transport command try to load a
        // queue that no longer exists.
        inner.pending_materialize = false;
        drop(inner);
        self.mpv.stop();
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

    /// Playback status without cloning the queue.
    ///
    /// [`state`](Self::state) deep-clones the whole `PlayerState`, queue
    /// included — far too much work for a caller that polls on a timer and
    /// only wants to know whether audio is running.
    pub fn status(&self) -> PlaybackStatus {
        self.inner.lock().state.status
    }

    /// The playing track's rating key, without cloning the queue. See
    /// [`status`](Self::status) for why this exists.
    pub fn current_track_key(&self) -> Option<String> {
        self.inner
            .lock()
            .state
            .current_track
            .as_ref()
            .map(|t| t.rating_key.clone())
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
}
