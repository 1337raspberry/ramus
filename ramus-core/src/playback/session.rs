//! Plex session timeline reporting and scrobble tracking. Determines what
//! and when to report; actual HTTP calls are handled by the caller.

use crate::models::{PlaybackStatus, Track};

/// Scrobble at >= 90% progress.
pub const SCROBBLE_THRESHOLD: f64 = 0.9;

/// Periodic report interval in seconds.
pub const REPORT_INTERVAL_SECS: u64 = 10;

/// Timeline state payload for Plex session reporting.
/// Positions are in milliseconds (Plex API convention).
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineState {
    pub rating_key: String,
    pub state: PlaybackStatus,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub play_session_id: String,
}

impl TimelineState {
    /// Progress fraction 0.0..1.0.
    pub fn progress(&self) -> f64 {
        if self.duration_ms <= 0 {
            return 0.0;
        }
        self.position_ms as f64 / self.duration_ms as f64
    }

    /// Whether this position has passed the scrobble threshold.
    pub fn should_scrobble(&self) -> bool {
        self.progress() >= SCROBBLE_THRESHOLD
    }
}

/// Tracks session reporting state for Plex timeline updates.
///
/// Call methods on state transitions and periodically during playback. Use
/// the returned `TimelineState` to send reports to Plex, and the optional
/// scrobble key to trigger scrobble API calls.
pub struct SessionTracker {
    active_track_key: Option<String>,
    active_session_id: Option<String>,
    scrobbled_key: Option<String>,
    position: f64,
    duration: f64,
}

impl SessionTracker {
    pub fn new() -> Self {
        Self {
            active_track_key: None,
            active_session_id: None,
            scrobbled_key: None,
            position: 0.0,
            duration: 0.0,
        }
    }

    /// Register a new track starting. Returns the timeline state to report.
    pub fn track_started(&mut self, track: &Track, session_id: &str) -> TimelineState {
        self.active_track_key = Some(track.rating_key.clone());
        self.active_session_id = Some(session_id.to_string());
        self.scrobbled_key = None;
        self.position = 0.0;
        self.duration = track.duration;

        TimelineState {
            rating_key: track.rating_key.clone(),
            state: PlaybackStatus::Playing,
            position_ms: 0,
            duration_ms: (track.duration * 1000.0) as i64,
            play_session_id: session_id.to_string(),
        }
    }

    /// Update position (called periodically or on seek). Returns a timeline
    /// plus an optional scrobble rating key if the threshold was crossed.
    pub fn update_position(
        &mut self,
        position: f64,
        duration: f64,
    ) -> Option<(TimelineState, Option<String>)> {
        self.position = position;
        self.duration = duration;

        let (track_key, session_id) = match (&self.active_track_key, &self.active_session_id) {
            (Some(k), Some(s)) => (k.clone(), s.clone()),
            _ => return None,
        };

        let timeline = TimelineState {
            rating_key: track_key.clone(),
            state: PlaybackStatus::Playing,
            position_ms: (position * 1000.0) as i64,
            duration_ms: (duration * 1000.0) as i64,
            play_session_id: session_id,
        };

        let scrobble =
            if timeline.should_scrobble() && self.scrobbled_key.as_deref() != Some(&track_key) {
                self.scrobbled_key = Some(track_key.clone());
                Some(track_key)
            } else {
                None
            };

        Some((timeline, scrobble))
    }

    /// Returns the paused timeline.
    pub fn playback_paused(&self) -> Option<TimelineState> {
        let (track_key, session_id) = match (&self.active_track_key, &self.active_session_id) {
            (Some(k), Some(s)) => (k.clone(), s.clone()),
            _ => return None,
        };

        Some(TimelineState {
            rating_key: track_key,
            state: PlaybackStatus::Paused,
            position_ms: (self.position * 1000.0) as i64,
            duration_ms: (self.duration * 1000.0) as i64,
            play_session_id: session_id,
        })
    }

    /// Returns the playing timeline on resume.
    pub fn playback_resumed(&self) -> Option<TimelineState> {
        let (track_key, session_id) = match (&self.active_track_key, &self.active_session_id) {
            (Some(k), Some(s)) => (k.clone(), s.clone()),
            _ => return None,
        };

        Some(TimelineState {
            rating_key: track_key,
            state: PlaybackStatus::Playing,
            position_ms: (self.position * 1000.0) as i64,
            duration_ms: (self.duration * 1000.0) as i64,
            play_session_id: session_id,
        })
    }

    /// Returns the stopped timeline and clears active session state.
    pub fn playback_stopped(&mut self) -> Option<TimelineState> {
        let (track_key, session_id) = match (&self.active_track_key, &self.active_session_id) {
            (Some(k), Some(s)) => (k.clone(), s.clone()),
            _ => return None,
        };

        let timeline = TimelineState {
            rating_key: track_key,
            state: PlaybackStatus::Stopped,
            position_ms: (self.position * 1000.0) as i64,
            duration_ms: (self.duration * 1000.0) as i64,
            play_session_id: session_id,
        };

        self.active_track_key = None;
        self.active_session_id = None;

        Some(timeline)
    }

    /// Returns an immediate position update timeline after a user seek.
    pub fn playback_seeked(&mut self, position: f64) -> Option<TimelineState> {
        self.position = position;

        let (track_key, session_id) = match (&self.active_track_key, &self.active_session_id) {
            (Some(k), Some(s)) => (k.clone(), s.clone()),
            _ => return None,
        };

        Some(TimelineState {
            rating_key: track_key,
            state: PlaybackStatus::Playing,
            position_ms: (position * 1000.0) as i64,
            duration_ms: (self.duration * 1000.0) as i64,
            play_session_id: session_id,
        })
    }

    /// Whether there's an active session being tracked.
    pub fn has_active_session(&self) -> bool {
        self.active_track_key.is_some()
    }

    /// Current active track rating key, if any.
    pub fn active_track_key(&self) -> Option<&str> {
        self.active_track_key.as_deref()
    }

    /// Whether the given track should be scrobbled: >= 90% progress and
    /// not already scrobbled this session.
    pub fn should_scrobble_track(&self, rating_key: &str) -> bool {
        let progress = if self.duration > 0.0 {
            self.position / self.duration
        } else {
            0.0
        };
        progress >= SCROBBLE_THRESHOLD && self.scrobbled_key.as_deref() != Some(rating_key)
    }

    /// Record that a track has been scrobbled to prevent double-scrobbling.
    pub fn mark_scrobbled(&mut self, rating_key: String) {
        self.scrobbled_key = Some(rating_key);
    }
}

impl Default for SessionTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_track(key: &str, duration: f64) -> Track {
        Track {
            rating_key: key.into(),
            title: "Test".into(),
            artist_name: "Artist".into(),
            track_artist: None,
            album_title: "Album".into(),
            album_key: None,
            index: None,
            duration,
            codec: None,
            part_key: None,
            thumb: None,
            is_favourite: false,
            bitrate: None,
            disc_number: None,
        }
    }

    #[test]
    fn test_track_started_returns_playing_timeline() {
        let mut tracker = SessionTracker::new();
        let track = make_track("123", 240.0);
        let tl = tracker.track_started(&track, "session-1");

        assert_eq!(tl.rating_key, "123");
        assert_eq!(tl.state, PlaybackStatus::Playing);
        assert_eq!(tl.position_ms, 0);
        assert_eq!(tl.duration_ms, 240000);
        assert_eq!(tl.play_session_id, "session-1");
        assert!(tracker.has_active_session());
    }

    #[test]
    fn test_scrobble_at_90_percent() {
        let mut tracker = SessionTracker::new();
        let track = make_track("123", 100.0);
        tracker.track_started(&track, "s1");

        let (_, scrobble) = tracker.update_position(89.0, 100.0).unwrap();
        assert!(scrobble.is_none());

        let (_, scrobble) = tracker.update_position(90.0, 100.0).unwrap();
        assert_eq!(scrobble, Some("123".to_string()));

        let (_, scrobble) = tracker.update_position(95.0, 100.0).unwrap();
        assert!(scrobble.is_none());
    }

    #[test]
    fn test_new_track_resets_scrobble() {
        let mut tracker = SessionTracker::new();
        let track1 = make_track("123", 100.0);
        tracker.track_started(&track1, "s1");

        let (_, scrobble) = tracker.update_position(95.0, 100.0).unwrap();
        assert!(scrobble.is_some());

        let track2 = make_track("456", 200.0);
        tracker.track_started(&track2, "s1");

        let (_, scrobble) = tracker.update_position(185.0, 200.0).unwrap();
        assert_eq!(scrobble, Some("456".to_string()));
    }

    #[test]
    fn test_pause_returns_paused_state() {
        let mut tracker = SessionTracker::new();
        let track = make_track("123", 100.0);
        tracker.track_started(&track, "s1");
        tracker.update_position(50.0, 100.0);

        let tl = tracker.playback_paused().unwrap();
        assert_eq!(tl.state, PlaybackStatus::Paused);
        assert_eq!(tl.position_ms, 50000);
    }

    #[test]
    fn test_resume_returns_playing_state() {
        let mut tracker = SessionTracker::new();
        let track = make_track("123", 100.0);
        tracker.track_started(&track, "s1");
        tracker.update_position(50.0, 100.0);

        let tl = tracker.playback_resumed().unwrap();
        assert_eq!(tl.state, PlaybackStatus::Playing);
        assert_eq!(tl.position_ms, 50000);
    }

    #[test]
    fn test_stop_clears_session() {
        let mut tracker = SessionTracker::new();
        let track = make_track("123", 100.0);
        tracker.track_started(&track, "s1");

        assert!(tracker.has_active_session());
        let tl = tracker.playback_stopped().unwrap();
        assert_eq!(tl.state, PlaybackStatus::Stopped);
        assert!(!tracker.has_active_session());

        assert!(tracker.playback_stopped().is_none());
    }

    #[test]
    fn test_no_active_session_returns_none() {
        let tracker = SessionTracker::new();
        assert!(tracker.playback_paused().is_none());
        assert!(tracker.playback_resumed().is_none());
        assert_eq!(tracker.active_track_key(), None);
        assert!(!tracker.has_active_session());
    }

    #[test]
    fn test_update_without_session_returns_none() {
        let mut tracker = SessionTracker::new();
        assert!(tracker.update_position(10.0, 100.0).is_none());
    }

    #[test]
    fn test_timeline_progress() {
        let tl = TimelineState {
            rating_key: "1".into(),
            state: PlaybackStatus::Playing,
            position_ms: 45000,
            duration_ms: 100000,
            play_session_id: "s".into(),
        };
        assert!((tl.progress() - 0.45).abs() < 0.001);
        assert!(!tl.should_scrobble());
    }

    #[test]
    fn test_timeline_scrobble_threshold() {
        let tl = TimelineState {
            rating_key: "1".into(),
            state: PlaybackStatus::Playing,
            position_ms: 92000,
            duration_ms: 100000,
            play_session_id: "s".into(),
        };
        assert!(tl.should_scrobble());
    }

    #[test]
    fn test_zero_duration_progress() {
        let tl = TimelineState {
            rating_key: "1".into(),
            state: PlaybackStatus::Playing,
            position_ms: 5000,
            duration_ms: 0,
            play_session_id: "s".into(),
        };
        assert_eq!(tl.progress(), 0.0);
        assert!(!tl.should_scrobble());
    }

    #[test]
    fn test_positions_in_milliseconds() {
        let mut tracker = SessionTracker::new();
        let track = make_track("1", 180.5);
        let tl = tracker.track_started(&track, "s");
        assert_eq!(tl.duration_ms, 180500);

        tracker.update_position(60.25, 180.5);
        let tl = tracker.playback_paused().unwrap();
        assert_eq!(tl.position_ms, 60250);
    }

    #[test]
    fn test_seeked_updates_position() {
        let mut tracker = SessionTracker::new();
        let track = make_track("123", 200.0);
        tracker.track_started(&track, "s1");

        let tl = tracker.playback_seeked(90.0).unwrap();
        assert_eq!(tl.state, PlaybackStatus::Playing);
        assert_eq!(tl.position_ms, 90000);
        assert_eq!(tl.duration_ms, 200000);

        let tl = tracker.playback_paused().unwrap();
        assert_eq!(tl.position_ms, 90000);
    }

    #[test]
    fn test_seeked_without_session_returns_none() {
        let mut tracker = SessionTracker::new();
        assert!(tracker.playback_seeked(50.0).is_none());
    }

    #[test]
    fn test_active_track_key() {
        let mut tracker = SessionTracker::new();
        assert_eq!(tracker.active_track_key(), None);

        let track = make_track("abc", 100.0);
        tracker.track_started(&track, "s");
        assert_eq!(tracker.active_track_key(), Some("abc"));
    }
}
