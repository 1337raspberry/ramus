//! The failover ladder: reload-at-position, the recovery hold, and the
//! playlist resweep that keeps queued entries addressing a live connection.

use std::time::{Duration, Instant};

use crate::models::PlaybackStatus;

use super::diagnostics::{derive_phase, Phase};
use super::resolve::{resolve_url, resolve_url_with_resume, stream_record_option_for, ResumePlan};
use super::AudioPlayer;

/// Minimum gap between two *automatic* current-track reloads (connection
/// failover or file-ended recovery). Three uncoordinated triggers — the iOS
/// network-path monitor, the stall watchdog, and prefetch's failure counter —
/// can otherwise fire back-to-back and reload the same track several times for
/// one hiccup. User-initiated seeks/`previous` bypass this (they call
/// `reload_current_track` directly and must stay responsive).
pub(super) const RELOAD_COOLDOWN: Duration = Duration::from_secs(6);

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
pub(super) const RELOAD_SETTLE_WINDOW: Duration = Duration::from_millis(1500);

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

impl AudioPlayer {
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

        // Rewriting an entry below the playing index momentarily shifts
        // mpv's playlist-pos (remove decrements it, the re-insert restores
        // it) — some platforms report that churn as pos-change events. Arm
        // the reload settle window so `handle_playlist_pos_change` doesn't
        // process them as phantom track advances; landing back on the
        // current index (or the window expiring) closes it. Usually the
        // accompanying `force_reload_current_track` arms this anyway, but
        // it can decline (cooldown, fully-drained source) while the rewrite
        // still runs.
        {
            let mut inner = self.inner.lock();
            let current_idx = inner.state.queue_index;
            if rewrites.iter().any(|(idx, _, _)| *idx < current_idx) {
                inner.reloading_pos = Some(current_idx);
                inner.reload_started_at = Some(Instant::now());
            }
        }

        for (idx, new_url, opts) in rewrites.iter().rev() {
            self.mpv.playlist_remove(*idx as i64);
            self.mpv.load_file_at(new_url, *idx as i64, opts.as_deref());
        }
    }

    /// Attempt to recover from a failed track load by resuming it at the last
    /// known position over the current server connection. A network track's
    /// first failure resumes ([`RecoverOutcome::Reloading`]); a second failure
    /// on the same track, or one inside the reload cooldown, holds at position
    /// ([`RecoverOutcome::Held`]) rather than thrash or reset. A local file
    /// that failed to decode yields [`RecoverOutcome::Skipped`] via the caller.
    pub(super) fn try_recover_current_track(&self) -> RecoverOutcome {
        // Capture, guard, and stamp in ONE lock window so idx/track/resume
        // stay consistent; `expected_idx` then lets reload_current_track
        // decline if a user skip lands before it re-acquires the lock.
        let (resume, idx, serve_cached) = {
            let persistent = self.persistent_cache.read();
            let mut inner = self.inner.lock();
            let idx = inner.state.queue_index;
            let Some(track) = inner.state.queue.get(idx) else {
                return RecoverOutcome::Skipped;
            };
            let rating_key = track.rating_key.clone();
            // Persistent downloads: a file-ended error is a genuine
            // decode/file problem, not a transient stream drop — let the
            // caller skip.
            if persistent.contains_key(&rating_key) {
                return RecoverOutcome::Skipped;
            }
            // An LRU cache hit is ambiguous: the track may have been playing
            // from the cached file (decode failure → skip), or the prefetch
            // may have landed a copy mid-play while mpv was streaming from
            // the network — in which case this is a *network* failure with a
            // complete local copy sitting right there, and the recovery
            // should serve it instead of skipping a playable track. The two
            // are told apart by whether the cache entry postdates the
            // current load.
            let mut serve_cached = false;
            if inner.cache.get(&rating_key).is_some() {
                let landed_mid_play = match (
                    inner.cache.inserted_at(&rating_key),
                    inner.load_started_at,
                ) {
                    (Some(ins), Some(load)) => ins > load,
                    _ => false,
                };
                if !landed_mid_play {
                    return RecoverOutcome::Skipped;
                }
                serve_cached = true;
            }
            // Hold (don't thrash/reset) if we already retried this track, or
            // if the last automatic reload was too recent. Applies to the
            // serve-cached path too: a cached copy that itself fails to load
            // (e.g. a truncated file) must not reload-loop.
            if inner.last_retried_track.as_deref() == Some(rating_key.as_str())
                || inner.within_reload_cooldown()
            {
                return RecoverOutcome::Held(inner.position);
            }
            inner.last_retried_track = Some(rating_key);
            inner.last_auto_reload_at = Some(Instant::now());
            (inner.position, idx, serve_cached)
        };
        // Resume at the captured position (transcode `offset=` / direct-play
        // `start=` per `reload_current_track`), never a restart from 0:00.
        if self.reload_current_track_impl(Some(resume), Some(idx), serve_cached) {
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
    pub(super) fn reload_current_track(
        &self,
        resume: Option<f64>,
        expected_idx: Option<usize>,
    ) -> bool {
        self.reload_current_track_impl(resume, expected_idx, false)
    }

    /// `serve_cached: true` lifts the local-URL declines — used by the
    /// recovery path when a prefetch landed a cached copy *mid-play* and the
    /// failed network stream should be replaced by the local file (which
    /// `resolve_url_with_resume` then serves with an mpv `start=` seek).
    /// Every other caller keeps `false`: for a track already playing locally
    /// a reload is pointless (a server change can't affect a `file://` URL).
    fn reload_current_track_impl(
        &self,
        resume: Option<f64>,
        expected_idx: Option<usize>,
        serve_cached: bool,
    ) -> bool {
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
            if !serve_cached {
                if persistent.contains_key(&track.rating_key) {
                    return false;
                }
                if inner.cache.get(&track.rating_key).is_some() {
                    return false;
                }
            }
            let Some((url, plan)) =
                resolve_url_with_resume(&track, &inner, &persistent, resume)
            else {
                return false;
            };
            if !serve_cached && url.starts_with("file://") {
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
            // A reload replaces the stream, and the swap itself costs a gap.
            // Judge the fresh stream on its own evidence rather than letting
            // the reload's own silence read as further starvation.
            inner.starvation.clear();
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
        // A fully-drained source already holds every byte it will ever need
        // in mpv's demuxer cache — the dead upstream can't hurt it, and
        // reloading would cut clean audio for nothing (the reload itself
        // then has to fetch those bytes again over the new connection).
        if self.current_source_fully_drained() {
            log::debug!("force_reload_current_track: source fully drained, no reload needed");
            return false;
        }
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
}
