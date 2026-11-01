use serde::{Deserialize, Serialize};

use super::WidgetColors;
use crate::models::PlaybackStatus;

/// Version of the snapshot file layout. The widget treats any other value
/// as no snapshot at all.
pub const SNAPSHOT_SCHEMA: u32 = 1;

/// Number of waveform values a snapshot carries.
pub const WAVEFORM_POINTS: usize = 96;

/// How far a position may move before a paused snapshot counts as new
/// activity (a seek) rather than the same pause written again.
const SAME_POSITION_TOLERANCE_SEC: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WidgetPlayState {
    Playing,
    Paused,
    Idle,
    SignedOut,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetTrack {
    pub rating_key: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_sec: f64,
}

/// What the widget's Resume action would pick up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetResume {
    pub title: String,
    pub artist: String,
}

/// Everything the home-screen widget renders from. Written by the app on
/// every playback state change; the widget plans its own later states
/// (paused once the track should have ended, idle after a long pause) from
/// this alone, so it stays truthful when the app is no longer running.
/// Times are seconds since the Unix epoch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetSnapshot {
    pub schema: u32,
    pub revision: u64,
    pub state: WidgetPlayState,
    pub track: Option<WidgetTrack>,
    pub position_sec: f64,
    pub position_at: f64,
    pub last_activity_at: f64,
    pub resumable: bool,
    pub resume: Option<WidgetResume>,
    pub colors: Option<WidgetColors>,
    pub waveform: Option<Vec<f32>>,
    pub cast: Option<String>,
}

/// Playback facts a snapshot is built from.
#[derive(Debug, Clone)]
pub struct SnapshotInput {
    pub signed_in: bool,
    pub status: PlaybackStatus,
    pub track: Option<WidgetTrack>,
    pub position_sec: f64,
    /// Whether a queue exists that playback could continue from.
    pub has_queue: bool,
    /// The "resume queue on launch" setting.
    pub resume_enabled: bool,
    pub colors: Option<WidgetColors>,
    pub waveform: Option<Vec<f32>>,
    /// Name of the player being cast to, if any.
    pub cast_player: Option<String>,
    pub now: f64,
}

/// Build the next snapshot. `prev` is the last one written, used to number
/// this one and to tell a repeated pause from a new one: while paused,
/// `last_activity_at` only moves when the state, the track or the position
/// changes, so rewriting the same pause doesn't postpone the switch to idle.
pub fn build_snapshot(input: SnapshotInput, prev: Option<&WidgetSnapshot>) -> WidgetSnapshot {
    let track = if input.signed_in { input.track } else { None };
    let state = match (&track, input.signed_in, input.status) {
        (_, false, _) => WidgetPlayState::SignedOut,
        (None, true, _) => WidgetPlayState::Idle,
        (Some(_), true, PlaybackStatus::Playing) => WidgetPlayState::Playing,
        (Some(_), true, _) => WidgetPlayState::Paused,
    };
    let position_sec = if track.is_some() {
        input.position_sec.max(0.0)
    } else {
        0.0
    };

    let unchanged = prev.is_some_and(|p| {
        p.state == state
            && p.track.as_ref().map(|t| &t.rating_key) == track.as_ref().map(|t| &t.rating_key)
            && (p.position_sec - position_sec).abs() < SAME_POSITION_TOLERANCE_SEC
    });
    let last_activity_at = match prev {
        Some(p) if state != WidgetPlayState::Playing && unchanged => p.last_activity_at,
        _ => input.now,
    };

    let resumable = input.signed_in && input.has_queue && input.resume_enabled && track.is_some();
    let resume = track.as_ref().filter(|_| resumable).map(|t| WidgetResume {
        title: t.title.clone(),
        artist: t.artist.clone(),
    });

    WidgetSnapshot {
        schema: SNAPSHOT_SCHEMA,
        revision: prev.map_or(1, |p| p.revision + 1),
        state,
        position_sec,
        position_at: input.now,
        last_activity_at,
        resumable,
        resume,
        colors: track.as_ref().and(input.colors),
        waveform: track.as_ref().and(input.waveform),
        cast: track.as_ref().and(input.cast_player),
        track,
    }
}

/// How far a playing position may drift from the previous snapshot's own
/// extrapolation before the widget needs a fresh one.
const SEEK_TOLERANCE_SEC: f64 = 2.0;

/// Whether `next` differs from the last snapshot written in a way the
/// widget would show. Steady playback doesn't (the widget extrapolates the
/// position itself), and neither does a stall, where time moves on but the
/// position doesn't, so the frequent state re-pushes made while playback is
/// stalled don't each rewrite the file and reload the widget.
pub fn needs_publish(prev: Option<&WidgetSnapshot>, next: &WidgetSnapshot) -> bool {
    let Some(prev) = prev else { return true };
    if prev.schema != next.schema
        || prev.state != next.state
        || prev.track != next.track
        || prev.resumable != next.resumable
        || prev.resume != next.resume
        || prev.colors != next.colors
        || prev.waveform != next.waveform
        || prev.cast != next.cast
    {
        return true;
    }
    match next.state {
        WidgetPlayState::Playing => {
            let expected = prev.position_sec + (next.position_at - prev.position_at).max(0.0);
            let moved =
                (next.position_sec - prev.position_sec).abs() >= SAME_POSITION_TOLERANCE_SEC;
            // Behind the widget's extrapolation only counts once the
            // position has moved off the written one: a seek back, or
            // playback flowing again after a stall. A position frozen by
            // the stall itself doesn't.
            next.position_sec > expected + SEEK_TOLERANCE_SEC
                || (moved && next.position_sec < expected - SEEK_TOLERANCE_SEC)
        }
        WidgetPlayState::Paused => {
            (next.position_sec - prev.position_sec).abs() >= SAME_POSITION_TOLERANCE_SEC
        }
        WidgetPlayState::Idle | WidgetPlayState::SignedOut => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(key: &str) -> WidgetTrack {
        WidgetTrack {
            rating_key: key.into(),
            title: "Tessellate".into(),
            artist: "alt-J".into(),
            album: "An Awesome Wave".into(),
            duration_sec: 182.7,
        }
    }

    fn input(status: PlaybackStatus, now: f64) -> SnapshotInput {
        SnapshotInput {
            signed_in: true,
            status,
            track: Some(track("9")),
            position_sec: 69.0,
            has_queue: true,
            resume_enabled: true,
            colors: None,
            waveform: None,
            cast_player: None,
            now,
        }
    }

    #[test]
    fn signed_out_has_no_track() {
        let s = build_snapshot(
            SnapshotInput {
                signed_in: false,
                ..input(PlaybackStatus::Playing, 10.0)
            },
            None,
        );
        assert_eq!(s.state, WidgetPlayState::SignedOut);
        assert!(s.track.is_none());
        assert!(!s.resumable);
        assert!(s.resume.is_none());
    }

    #[test]
    fn no_track_is_idle() {
        let s = build_snapshot(
            SnapshotInput {
                track: None,
                ..input(PlaybackStatus::Stopped, 10.0)
            },
            None,
        );
        assert_eq!(s.state, WidgetPlayState::Idle);
        assert!(!s.resumable);
    }

    #[test]
    fn stopped_with_track_is_paused() {
        let s = build_snapshot(input(PlaybackStatus::Stopped, 10.0), None);
        assert_eq!(s.state, WidgetPlayState::Paused);
        assert!(s.resumable);
        assert_eq!(
            s.resume.as_ref().map(|r| r.title.as_str()),
            Some("Tessellate")
        );
    }

    #[test]
    fn revision_increments() {
        let a = build_snapshot(input(PlaybackStatus::Playing, 10.0), None);
        assert_eq!(a.revision, 1);
        let b = build_snapshot(input(PlaybackStatus::Paused, 11.0), Some(&a));
        assert_eq!(b.revision, 2);
        assert_eq!(b.schema, SNAPSHOT_SCHEMA);
    }

    #[test]
    fn paused_keeps_last_activity_when_unchanged() {
        let prev = build_snapshot(input(PlaybackStatus::Paused, 100.0), None);
        let next = build_snapshot(input(PlaybackStatus::Paused, 200.0), Some(&prev));
        assert_eq!(next.last_activity_at, 100.0);
        assert_eq!(next.position_at, 200.0);
    }

    #[test]
    fn paused_resets_last_activity_on_seek() {
        let prev = build_snapshot(input(PlaybackStatus::Paused, 100.0), None);
        let seeked = SnapshotInput {
            position_sec: 99.0,
            ..input(PlaybackStatus::Paused, 200.0)
        };
        assert_eq!(build_snapshot(seeked, Some(&prev)).last_activity_at, 200.0);
        let other = SnapshotInput {
            track: Some(track("10")),
            ..input(PlaybackStatus::Paused, 200.0)
        };
        assert_eq!(build_snapshot(other, Some(&prev)).last_activity_at, 200.0);
    }

    #[test]
    fn playing_always_refreshes_last_activity() {
        let prev = build_snapshot(input(PlaybackStatus::Playing, 100.0), None);
        let next = build_snapshot(input(PlaybackStatus::Playing, 150.0), Some(&prev));
        assert_eq!(next.last_activity_at, 150.0);
        assert_eq!(next.state, WidgetPlayState::Playing);
    }

    #[test]
    fn resume_off_means_not_resumable() {
        let s = build_snapshot(
            SnapshotInput {
                resume_enabled: false,
                ..input(PlaybackStatus::Paused, 1.0)
            },
            None,
        );
        assert_eq!(s.state, WidgetPlayState::Paused);
        assert!(!s.resumable);
        assert!(s.resume.is_none());
        let empty = build_snapshot(
            SnapshotInput {
                has_queue: false,
                ..input(PlaybackStatus::Paused, 1.0)
            },
            None,
        );
        assert!(!empty.resumable);
    }

    #[test]
    fn json_field_names_are_camel_case() {
        let s = build_snapshot(
            SnapshotInput {
                cast_player: Some("Living Room".into()),
                ..input(PlaybackStatus::Playing, 5.0)
            },
            None,
        );
        let json = serde_json::to_string(&s).unwrap();
        for key in [
            "\"positionAt\"",
            "\"lastActivityAt\"",
            "\"ratingKey\"",
            "\"durationSec\"",
            "\"positionSec\"",
            "\"state\":\"playing\"",
            "\"cast\":\"Living Room\"",
        ] {
            assert!(json.contains(key), "{key} missing from {json}");
        }
        let back: WidgetSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn needs_publish_first_and_on_state_change() {
        let a = build_snapshot(input(PlaybackStatus::Playing, 100.0), None);
        assert!(needs_publish(None, &a));
        let paused = build_snapshot(input(PlaybackStatus::Paused, 101.0), Some(&a));
        assert!(needs_publish(Some(&a), &paused));
        let other = build_snapshot(
            SnapshotInput {
                track: Some(track("10")),
                ..input(PlaybackStatus::Playing, 101.0)
            },
            Some(&a),
        );
        assert!(needs_publish(Some(&a), &other));
    }

    #[test]
    fn needs_publish_ignores_steady_playback_and_stalls() {
        let a = build_snapshot(input(PlaybackStatus::Playing, 100.0), None);
        // 10 s later, 10 s further on: the widget's own extrapolation.
        let steady = build_snapshot(
            SnapshotInput {
                position_sec: 79.0,
                ..input(PlaybackStatus::Playing, 110.0)
            },
            Some(&a),
        );
        assert!(!needs_publish(Some(&a), &steady));
        // A stall: time moves, position doesn't.
        let stalled = build_snapshot(input(PlaybackStatus::Playing, 110.0), Some(&a));
        assert!(!needs_publish(Some(&a), &stalled));
    }

    #[test]
    fn needs_publish_on_a_backward_seek_after_an_early_write() {
        // Written at the start of the track; 100 s later the listener seeks
        // back to 55 s, which is ahead of the written position but well
        // behind where the widget has extrapolated to.
        let a = build_snapshot(
            SnapshotInput {
                position_sec: 0.0,
                ..input(PlaybackStatus::Playing, 100.0)
            },
            None,
        );
        let back = build_snapshot(
            SnapshotInput {
                position_sec: 55.0,
                ..input(PlaybackStatus::Playing, 200.0)
            },
            Some(&a),
        );
        assert!(needs_publish(Some(&a), &back));
    }

    #[test]
    fn needs_publish_when_playback_moves_again_after_a_stall() {
        // Written at 69 s; playback stalled at 79 s for a minute and a half
        // and is now moving again, far behind the widget's extrapolation.
        let a = build_snapshot(input(PlaybackStatus::Playing, 100.0), None);
        let resumed = build_snapshot(
            SnapshotInput {
                position_sec: 79.5,
                ..input(PlaybackStatus::Playing, 200.0)
            },
            Some(&a),
        );
        assert!(needs_publish(Some(&a), &resumed));
        // Once that is written, the frozen position of a further stall
        // doesn't write again.
        let frozen = build_snapshot(
            SnapshotInput {
                position_sec: 79.5,
                ..input(PlaybackStatus::Playing, 230.0)
            },
            Some(&resumed),
        );
        assert!(!needs_publish(Some(&resumed), &frozen));
    }

    #[test]
    fn needs_publish_on_seek() {
        let a = build_snapshot(input(PlaybackStatus::Playing, 100.0), None);
        let back = build_snapshot(
            SnapshotInput {
                position_sec: 20.0,
                ..input(PlaybackStatus::Playing, 101.0)
            },
            Some(&a),
        );
        assert!(needs_publish(Some(&a), &back));
        let forward = build_snapshot(
            SnapshotInput {
                position_sec: 150.0,
                ..input(PlaybackStatus::Playing, 101.0)
            },
            Some(&a),
        );
        assert!(needs_publish(Some(&a), &forward));
        let p = build_snapshot(input(PlaybackStatus::Paused, 100.0), None);
        let paused_seek = build_snapshot(
            SnapshotInput {
                position_sec: 120.0,
                ..input(PlaybackStatus::Paused, 101.0)
            },
            Some(&p),
        );
        assert!(needs_publish(Some(&p), &paused_seek));
    }

    #[test]
    fn needs_publish_when_colours_or_waveform_arrive() {
        let a = build_snapshot(input(PlaybackStatus::Paused, 100.0), None);
        let wave = build_snapshot(
            SnapshotInput {
                waveform: Some(vec![0.5]),
                ..input(PlaybackStatus::Paused, 101.0)
            },
            Some(&a),
        );
        assert!(needs_publish(Some(&a), &wave));
    }
}
