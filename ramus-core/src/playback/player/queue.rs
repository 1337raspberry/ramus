//! Queue mutation and navigation: loading, appending, inserting, removing,
//! and the deliberate skips.

use crate::models::{PlaybackStatus, Track};
use crate::playback::mpv::LoadMode;

use super::resolve::{resolve_url, stream_record_option_for};
use super::AudioPlayer;

/// Threshold in seconds: if position > this, `previous()` restarts instead of going back.
const PREVIOUS_RESTART_THRESHOLD: f64 = 3.0;

impl AudioPlayer {
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
            // See `handle_playlist_pos_change`: playing from the cache is a
            // use, keep the LRU order honest.
            let rk = inner.state.queue[start_at].rating_key.clone();
            inner.cache.touch(&rk);
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
            inner.begin_load();
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
        inner.begin_load();
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
        inner.begin_load();
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
        inner.begin_load();
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
}
