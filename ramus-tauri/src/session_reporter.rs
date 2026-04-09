//! Plex session reporter. Orchestrates SessionTracker and PlexClient for
//! periodic timeline updates, scrobble detection, and graceful shutdown
//! reporting. All public methods are synchronous for use from mpv callbacks.

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
        })
    }

    /// Report that a new track started playing.
    pub fn track_started(&self, track: &Track, session_id: &str) {
        let timeline = self.tracker.lock().track_started(track, session_id);
        self.send_timeline(&timeline);
        self.start_periodic();
    }

    /// Report a track ended naturally (auto-advance) and scrobble it.
    pub fn track_ended(&self, track: &Track) {
        let rk = track.rating_key.clone();
        let client = self.client.clone();
        tauri::async_runtime::spawn(async move {
            client.scrobble(&rk).await;
        });
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
    }

    /// Report playback stopped (end of queue, new queue load, or user stop).
    pub fn playback_stopped(&self) {
        self.stop_periodic();
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

    // --- Internals ---

    fn update_tracker_position(&self) {
        let pos = self.player.position();
        let dur = self.player.duration();
        let _ = self.tracker.lock().update_position(pos, dur);
    }

    fn send_timeline(&self, tl: &TimelineState) {
        let client = self.client.clone();
        let rk = tl.rating_key.clone();
        let state_str = tl.state.as_plex_str().to_string();
        let time = tl.position_ms;
        let dur = tl.duration_ms;
        let sid = tl.play_session_id.clone();
        tauri::async_runtime::spawn(async move {
            client.report_timeline(&rk, &state_str, time, dur, &sid).await;
        });
    }

    fn start_periodic(&self) {
        *self.periodic_active.lock() = true;
        self.tick_notify.notify_one();
    }

    fn stop_periodic(&self) {
        *self.periodic_active.lock() = false;
    }

    /// Lazily spawn the periodic reporting loop. Must be called after
    /// Tauri's async runtime is available (i.e. after setup).
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
                let client = reporter.client.clone();
                tauri::async_runtime::spawn(async move {
                    client.scrobble(&rk).await;
                });
            }
        }
    }
}
