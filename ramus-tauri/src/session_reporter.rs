//! Plex session reporter. Orchestrates SessionTracker and PlexClient for
//! periodic timeline updates, scrobble detection (at >= 90% progress, once per
//! track), and graceful shutdown reporting. All public methods are synchronous
//! for use from mpv callbacks.

use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;

use ramus_core::models::Track;
use ramus_core::playback::player::AudioPlayer;
use ramus_core::playback::session::{SessionTracker, TimelineState, REPORT_INTERVAL_SECS};
use ramus_core::plex::client::PlexClient;

pub type ReporterRef = Arc<Mutex<Option<Arc<SessionReporter>>>>;

pub struct SessionReporter {
    tracker: Mutex<SessionTracker>,
    client: Arc<PlexClient>,
    player: Arc<AudioPlayer>,
    /// Wakes the periodic loop when reporting should (re)start.
    tick_notify: Arc<Notify>,
    /// Whether periodic reporting is currently active.
    periodic_active: Arc<Mutex<bool>>,
    /// Whether the periodic loop task has been spawned yet.
    loop_spawned: Mutex<bool>,
    /// Last rating_key reported via track_started; deduplicates the
    /// overlapping calls from play_tracks and on_playlist_pos_change.
    last_started_key: Mutex<Option<String>>,
    /// Scrobbles whose sends failed every in-task retry. Re-attempted at the
    /// next natural connectivity moment (track start, resume) — a scrobble is
    /// a permanent play-count mutation, so it shouldn't be lost to the very
    /// outage that interrupted the track it belongs to.
    failed_scrobbles: Arc<Mutex<Vec<String>>>,
}

impl SessionReporter {
    pub fn new(client: Arc<PlexClient>, player: Arc<AudioPlayer>) -> Arc<Self> {
        Arc::new(Self {
            tracker: Mutex::new(SessionTracker::new()),
            client,
            player,
            tick_notify: Arc::new(Notify::new()),
            periodic_active: Arc::new(Mutex::new(false)),
            loop_spawned: Mutex::new(false),
            last_started_key: Mutex::new(None),
            failed_scrobbles: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// Report that a new track started playing. Deduplicates if the same
    /// track is reported twice (play_tracks + on_playlist_pos_change overlap).
    pub fn track_started(&self, track: &Track, session_id: &str) {
        let mut last = self.last_started_key.lock();
        if last.as_deref() == Some(&track.rating_key) {
            return;
        }
        *last = Some(track.rating_key.clone());
        drop(last);

        let timeline = self.tracker.lock().track_started(track, session_id);
        self.send_timeline(&timeline);
        self.start_periodic();
        // A track starting is a natural connectivity moment — retry any
        // scrobbles a past outage stranded.
        self.flush_failed_scrobbles();
    }

    /// Close out one track and open the next in a single ordered step, using
    /// a position snapshot taken *before* the player state moved on. Covers
    /// natural advances, manual skips, and end-of-queue (`next` = None). The
    /// live player already reads as the next track at 0:00 by the time the
    /// advance event reaches the platform layer, so reading `position()`
    /// here would report every finish as `stopped` at time=0 and make the
    /// boundary scrobble check unreachable.
    pub fn track_transition(
        &self,
        prev: &Track,
        prev_pos: f64,
        prev_dur: f64,
        next: Option<&Track>,
        session_id: &str,
    ) {
        self.stop_periodic();
        *self.last_started_key.lock() = None;

        let (stopped, scrobble) = {
            let mut tracker = self.tracker.lock();
            // Guard against a desynced tracker (e.g. reporting was never
            // started for this track): closing out would scrobble whatever
            // stale key the tracker still holds at the wrong position.
            if tracker.active_track_key() == Some(prev.rating_key.as_str()) {
                let scrobble = tracker
                    .update_position(prev_pos, prev_dur)
                    .and_then(|(_, key)| key);
                (tracker.playback_stopped(), scrobble)
            } else {
                (None, None)
            }
        };

        if let Some(ref tl) = stopped {
            self.send_timeline(tl);
        }
        if let Some(rk) = scrobble {
            self.send_scrobble(rk);
        }
        if let Some(next) = next {
            self.track_started(next, session_id);
        }
    }

    /// Report playback paused.
    pub fn playback_paused(&self) {
        self.update_tracker_position();
        if let Some(timeline) = self.tracker.lock().playback_paused() {
            self.send_timeline(&timeline);
        }
        self.stop_periodic();
    }

    /// Report playback resumed from pause.
    pub fn playback_resumed(&self) {
        self.update_tracker_position();
        if let Some(timeline) = self.tracker.lock().playback_resumed() {
            self.send_timeline(&timeline);
        }
        self.start_periodic();
        // Resuming often follows a reconnect — retry stranded scrobbles.
        self.flush_failed_scrobbles();
    }

    /// Report playback stopped (end of queue, new queue load, or user stop).
    pub fn playback_stopped(&self) {
        self.stop_periodic();
        *self.last_started_key.lock() = None;
        self.update_tracker_position();
        if let Some(timeline) = self.tracker.lock().playback_stopped() {
            self.send_timeline(&timeline);
        }
    }

    /// Report a seek to a new position.
    pub fn playback_seeked(&self, position: f64) {
        if let Some(timeline) = self.tracker.lock().playback_seeked(position) {
            self.send_timeline(&timeline);
        }
    }

    /// Synchronous stop for app termination. Waits up to 2 seconds.
    pub fn stop_sync(&self) {
        self.stop_periodic();
        self.update_tracker_position();
        let timeline = self.tracker.lock().playback_stopped();
        if let Some(tl) = timeline {
            let client = self.client.clone();
            let state_str = tl.state.as_plex_str().to_string();
            let rk = tl.rating_key.clone();
            let time = tl.position_ms;
            let dur = tl.duration_ms;
            let sid = tl.play_session_id.clone();

            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let _ = handle.block_on(async {
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        client.report_timeline(&rk, &state_str, time, dur, &sid),
                    )
                    .await
                });
            }
        }
    }

    fn update_tracker_position(&self) {
        let pos = self.player.position();
        let dur = self.player.duration();
        let _ = self.tracker.lock().update_position(pos, dur);
    }

    /// Send a scrobble with in-task retries. Transient failures back off and
    /// re-send; a fully failed send parks the key in `failed_scrobbles` for
    /// the next flush rather than silently dropping the play count. The
    /// tracker's `scrobbled_key` was already marked by the caller — that
    /// stays deliberate: unmarking on failure would let the periodic loop
    /// re-yield the key while an earlier attempt may still land server-side,
    /// double-counting the play.
    pub fn send_scrobble(&self, rating_key: String) {
        let client = self.client.clone();
        let failed = self.failed_scrobbles.clone();
        tauri::async_runtime::spawn(async move {
            for delay_secs in [0u64, 2, 8] {
                if delay_secs > 0 {
                    tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                }
                if client.scrobble(&rating_key).await {
                    return;
                }
            }
            log::warn!("scrobble failed after retries; queued for later flush");
            let mut queue = failed.lock();
            if !queue.contains(&rating_key) {
                queue.push(rating_key);
            }
        });
    }

    /// Re-attempt every scrobble stranded by past send failures. Keys that
    /// fail again re-queue themselves via `send_scrobble`, so nothing is
    /// lost — the queue just waits for the next flush moment.
    pub fn flush_failed_scrobbles(&self) {
        let pending: Vec<String> = std::mem::take(&mut *self.failed_scrobbles.lock());
        for rk in pending {
            self.send_scrobble(rk);
        }
    }

    fn send_timeline(&self, tl: &TimelineState) {
        let client = self.client.clone();
        let rk = tl.rating_key.clone();
        let state_str = tl.state.as_plex_str().to_string();
        let time = tl.position_ms;
        let dur = tl.duration_ms;
        let sid = tl.play_session_id.clone();
        tauri::async_runtime::spawn(async move {
            client
                .report_timeline(&rk, &state_str, time, dur, &sid)
                .await;
        });
    }

    fn start_periodic(&self) {
        *self.periodic_active.lock() = true;
        self.tick_notify.notify_one();
    }

    fn stop_periodic(&self) {
        *self.periodic_active.lock() = false;
    }

    /// Lazily spawn the periodic reporting loop. Must be called after Tauri's
    /// async runtime is available (i.e. after setup).
    pub fn ensure_loop_spawned(self: &Arc<Self>) {
        let mut spawned = self.loop_spawned.lock();
        if !*spawned {
            *spawned = true;
            tauri::async_runtime::spawn(periodic_loop(
                Arc::downgrade(self),
                self.tick_notify.clone(),
                self.periodic_active.clone(),
            ));
        }
    }
}

async fn periodic_loop(
    reporter: Weak<SessionReporter>,
    notify: Arc<Notify>,
    active: Arc<Mutex<bool>>,
) {
    loop {
        if !*active.lock() {
            notify.notified().await;
            continue;
        }

        tokio::time::sleep(Duration::from_secs(REPORT_INTERVAL_SECS)).await;

        if !*active.lock() {
            continue;
        }

        let Some(reporter) = reporter.upgrade() else {
            break;
        };

        let pos = reporter.player.position();
        let dur = reporter.player.duration();

        let result = reporter.tracker.lock().update_position(pos, dur);
        if let Some((timeline, scrobble_key)) = result {
            reporter.send_timeline(&timeline);
            if let Some(rk) = scrobble_key {
                reporter.send_scrobble(rk);
            }
        }
    }
}
