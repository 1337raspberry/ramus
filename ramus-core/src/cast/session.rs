//! A cast's state on this side: the mirror of the server play queue, and
//! the reducer that turns each timeline poll into playback events.

use std::time::Duration;

use super::companion::PollOutcome;
use super::play_queue::PlayQueue;
use super::timeline::{MusicTimeline, RemoteState};
use crate::models::Track;
use crate::playback::session::SCROBBLE_THRESHOLD;

/// One item of the mirrored queue: the play-queue item ID and its track.
#[derive(Debug, Clone, PartialEq)]
pub struct CastEntry {
    pub item_id: i64,
    pub track: Track,
}

/// The server play queue as this device shows it, in queue order. Its
/// indexes are what `playback-state`'s `queueIndex` and the Up Next
/// commands refer to while casting.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CastQueue {
    pub play_queue_id: i64,
    pub version: i64,
    pub entries: Vec<CastEntry>,
}

impl CastQueue {
    /// Mirrors `queue`, resolving each rating key to a library track.
    /// Items the library doesn't hold are left out, as playlists do.
    pub fn resolve(queue: &PlayQueue, lookup: impl Fn(&str) -> Option<Track>) -> Self {
        Self {
            play_queue_id: queue.id,
            version: queue.version,
            entries: queue
                .items
                .iter()
                .filter_map(|item| {
                    lookup(&item.rating_key).map(|track| CastEntry {
                        item_id: item.item_id,
                        track,
                    })
                })
                .collect(),
        }
    }

    pub fn tracks(&self) -> Vec<Track> {
        self.entries.iter().map(|e| e.track.clone()).collect()
    }

    pub fn index_of_item(&self, item_id: i64) -> Option<usize> {
        self.entries.iter().position(|e| e.item_id == item_id)
    }

    pub fn item_at(&self, index: usize) -> Option<i64> {
        self.entries.get(index).map(|e| e.item_id)
    }

    /// `move_queue_item(from, to)` as the server's move: the item, and the
    /// item it should follow once moved (`None` = the top).
    pub fn move_target(&self, from: usize, to: usize) -> Option<(i64, Option<i64>)> {
        let len = self.entries.len();
        if from == to || from >= len || to >= len {
            return None;
        }
        let mut rest: Vec<i64> = self.entries.iter().map(|e| e.item_id).collect();
        let item = rest.remove(from);
        let after = if to == 0 { None } else { Some(rest[to - 1]) };
        Some((item, after))
    }
}

/// Failed polls in a row before the player counts as lost.
pub const LOST_AFTER_FAILURES: u32 = 3;
/// Polls that may still show the player's previous play queue (or none)
/// after a queue was sent, before that counts as the player having moved on.
pub const FOREIGN_GRACE_POLLS: u32 = 10;
pub const POLL_VISIBLE: Duration = Duration::from_secs(1);
pub const POLL_HIDDEN: Duration = Duration::from_secs(5);
pub const POLL_BACKOFF_MAX: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Connected,
    Lost,
}

/// What a poll means for this device.
#[derive(Debug, Clone, PartialEq)]
pub enum CastEffect {
    /// Status and mirror index to emit as `playback-state`.
    EmitState {
        status: &'static str,
        index: Option<usize>,
    },
    EmitPosition {
        position: f64,
        duration: f64,
    },
    EmitBuffering(bool),
    /// The server queue changed; refetch the mirror.
    RefetchQueue,
    /// Record a local play of this rating key.
    MarkPlayed(String),
    /// The player now plays a queue this cast didn't send.
    TakenOver,
    Link(Link),
}

/// Where local playback picks up when a cast ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LocalResume {
    Play { index: usize, position: f64 },
    Paused { index: usize, position: f64 },
}

impl LocalResume {
    pub fn paused(self) -> Self {
        match self {
            Self::Play { index, position } | Self::Paused { index, position } => {
                Self::Paused { index, position }
            }
        }
    }
}

/// The reducer behind a cast: each poll outcome in, the effects to run out.
/// Pure, so the runtime only performs what it's told.
#[derive(Debug, Clone, Default)]
pub struct CastState {
    last_status: Option<&'static str>,
    last_index: Option<usize>,
    buffering: bool,
    marked_item: Option<i64>,
    failures: u32,
    lost: bool,
    seen_ours: bool,
    foreign_polls: u32,
    taken_over: bool,
    last_timeline: Option<MusicTimeline>,
}

impl CastState {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new play queue was sent: forget everything about the previous one
    /// except the link's health.
    pub fn restart(&mut self) {
        *self = Self {
            failures: self.failures,
            lost: self.lost,
            ..Self::default()
        };
    }

    /// Makes the next poll emit its state again (a resync after the webview slept).
    pub fn forget_emitted(&mut self) {
        self.last_status = None;
        self.buffering = false;
    }

    pub fn link(&self) -> Link {
        if self.lost {
            Link::Lost
        } else {
            Link::Connected
        }
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    pub fn index(&self) -> Option<usize> {
        self.last_index
    }

    pub fn remote_state(&self) -> Option<RemoteState> {
        self.last_timeline.as_ref().map(|t| t.state)
    }

    fn locate(&self, timeline: &MusicTimeline, queue: &CastQueue) -> Option<usize> {
        if let Some(item) = timeline.play_queue_item_id {
            return queue.index_of_item(item);
        }
        let key = timeline.rating_key.as_deref()?;
        let from = self.last_index.unwrap_or(0);
        queue.entries[from.min(queue.entries.len())..]
            .iter()
            .position(|e| e.track.rating_key == key)
            .map(|p| p + from)
            .or_else(|| queue.entries.iter().position(|e| e.track.rating_key == key))
    }

    fn push_state(
        &mut self,
        out: &mut Vec<CastEffect>,
        status: &'static str,
        index: Option<usize>,
    ) {
        if self.last_status != Some(status) || self.last_index != index {
            self.last_status = Some(status);
            self.last_index = index;
            out.push(CastEffect::EmitState { status, index });
        }
    }

    pub fn apply(&mut self, outcome: &PollOutcome, queue: &CastQueue) -> Vec<CastEffect> {
        let mut out = Vec::new();
        let timeline = match outcome {
            PollOutcome::Failed => {
                self.failures += 1;
                if self.failures >= LOST_AFTER_FAILURES && !self.lost {
                    self.lost = true;
                    out.push(CastEffect::Link(Link::Lost));
                }
                return out;
            }
            PollOutcome::NoMusicTimeline => None,
            PollOutcome::Timeline(t) => Some(t),
        };
        self.failures = 0;
        if self.lost {
            self.lost = false;
            self.forget_emitted();
            out.push(CastEffect::Link(Link::Connected));
        }

        let queue_id = timeline.and_then(|t| t.play_queue_id);
        if queue_id == Some(queue.play_queue_id) {
            self.seen_ours = true;
        } else if !self.seen_ours {
            // The player may still show what it played before this queue
            // reached it; nothing it says yet is about this cast.
            self.foreign_polls += 1;
            if self.foreign_polls >= FOREIGN_GRACE_POLLS && !self.taken_over {
                self.taken_over = true;
                out.push(CastEffect::TakenOver);
            }
            return out;
        } else if queue_id.is_some() {
            if !self.taken_over {
                self.taken_over = true;
                out.push(CastEffect::TakenOver);
            }
            return out;
        }

        let located = timeline.and_then(|t| self.locate(t, queue));
        if let Some(t) = timeline {
            let unknown_item = t.play_queue_item_id.is_some() && located.is_none();
            let newer = t.play_queue_version.is_some_and(|v| v > queue.version);
            if unknown_item || newer {
                out.push(CastEffect::RefetchQueue);
            }
            self.last_timeline = Some(t.clone());
        }

        let (status, buffering) = match timeline.map(|t| t.state) {
            Some(RemoteState::Playing) => ("playing", false),
            Some(RemoteState::Buffering) => ("playing", true),
            Some(RemoteState::Paused) => ("paused", false),
            Some(RemoteState::Stopped) | None => ("stopped", false),
        };
        self.push_state(&mut out, status, located.or(self.last_index));
        if buffering != self.buffering {
            self.buffering = buffering;
            out.push(CastEffect::EmitBuffering(buffering));
        }

        if let (Some(t), Some(i)) = (timeline, located) {
            if t.state != RemoteState::Stopped {
                let entry = &queue.entries[i];
                let duration = if t.duration_ms > 0 {
                    t.duration_ms as f64 / 1000.0
                } else {
                    entry.track.duration
                };
                let position = t.time_ms as f64 / 1000.0;
                out.push(CastEffect::EmitPosition { position, duration });
                if duration > 0.0
                    && position >= duration * SCROBBLE_THRESHOLD
                    && self.marked_item != Some(entry.item_id)
                {
                    self.marked_item = Some(entry.item_id);
                    out.push(CastEffect::MarkPlayed(entry.track.rating_key.clone()));
                }
            }
        }
        out
    }

    /// Re-locates the current item in a refetched mirror.
    pub fn remap(&mut self, queue: &CastQueue) -> Vec<CastEffect> {
        let mut out = Vec::new();
        let (Some(timeline), Some(status)) = (self.last_timeline.clone(), self.last_status) else {
            return out;
        };
        if let Some(index) = self.locate(&timeline, queue) {
            self.push_state(&mut out, status, Some(index));
        }
        out
    }

    /// Where local playback resumes from the last timeline seen: playing if
    /// the player was, paused where it was paused, and from the start of the
    /// last track when it had stopped.
    pub fn local_resume(&self, queue: &CastQueue) -> Option<LocalResume> {
        let last = queue.entries.len().checked_sub(1)?;
        let timeline = self.last_timeline.as_ref();
        let index = timeline
            .and_then(|t| self.locate(t, queue))
            .or(self.last_index)
            .unwrap_or(0)
            .min(last);
        let position = timeline.map_or(0.0, |t| t.time_ms as f64 / 1000.0);
        Some(match timeline.map(|t| t.state) {
            Some(RemoteState::Playing | RemoteState::Buffering) => {
                LocalResume::Play { index, position }
            }
            Some(RemoteState::Paused) => LocalResume::Paused { index, position },
            _ => LocalResume::Paused {
                index,
                position: 0.0,
            },
        })
    }
}

/// The wait before the next poll: 1 s on screen, 5 s hidden, and doubling
/// from 1 s up to 30 s while the player is lost.
pub fn poll_delay(link: Link, failures: u32, hidden: bool) -> Duration {
    if link == Link::Lost {
        let doublings = failures.saturating_sub(LOST_AFTER_FAILURES).min(5);
        return Duration::from_secs(1u64 << doublings).min(POLL_BACKOFF_MAX);
    }
    if hidden {
        POLL_HIDDEN
    } else {
        POLL_VISIBLE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cast::play_queue::PlayQueueItem;

    pub(super) fn track(key: &str, duration: f64) -> Track {
        Track {
            rating_key: key.to_string(),
            title: format!("Track {key}"),
            artist_name: "Artist".to_string(),
            track_artist: None,
            album_title: "Album".to_string(),
            album_key: None,
            index: None,
            duration,
            codec: None,
            part_key: None,
            thumb: None,
            is_favourite: false,
            bitrate: None,
            disc_number: None,
            file_size_bytes: None,
            rating_count: None,
            view_count: None,
            last_viewed_at: None,
        }
    }

    pub(super) fn queue(entries: &[(i64, &str)]) -> CastQueue {
        CastQueue {
            play_queue_id: 9001,
            version: 3,
            entries: entries
                .iter()
                .map(|(item_id, key)| CastEntry {
                    item_id: *item_id,
                    track: track(key, 200.0),
                })
                .collect(),
        }
    }

    #[test]
    fn resolve_skips_tracks_the_library_lacks() {
        let pq = PlayQueue {
            id: 9001,
            version: 3,
            selected_item_id: None,
            selected_item_offset: None,
            total_count: None,
            items: vec![
                PlayQueueItem {
                    item_id: 1,
                    rating_key: "a".into(),
                },
                PlayQueueItem {
                    item_id: 2,
                    rating_key: "gone".into(),
                },
                PlayQueueItem {
                    item_id: 3,
                    rating_key: "c".into(),
                },
            ],
            exhausted: false,
        };
        let q = CastQueue::resolve(&pq, |k| (k != "gone").then(|| track(k, 1.0)));
        assert_eq!((q.play_queue_id, q.version), (9001, 3));
        let keys: Vec<String> = q.tracks().into_iter().map(|t| t.rating_key).collect();
        assert_eq!(keys, ["a", "c"]);
        assert_eq!(q.index_of_item(3), Some(1));
    }

    #[test]
    fn move_target_names_the_item_to_follow() {
        let q = queue(&[(1, "a"), (2, "b"), (3, "c"), (4, "d")]);
        // [a,b,c,d] move 0→2 = [b,c,a,d]: a follows c.
        assert_eq!(q.move_target(0, 2), Some((1, Some(3))));
        // move 3→1 = [a,d,b,c]: d follows a.
        assert_eq!(q.move_target(3, 1), Some((4, Some(1))));
        // move 2→0 = [c,a,b,d]: c to the top.
        assert_eq!(q.move_target(2, 0), Some((3, None)));
        assert_eq!(q.move_target(1, 1), None);
        assert_eq!(q.move_target(0, 9), None);
    }

    #[test]
    fn duplicate_tracks_are_addressed_by_item_id() {
        let q = queue(&[(1, "a"), (2, "b"), (3, "a")]);
        assert_eq!(q.index_of_item(3), Some(2));
        assert_eq!(q.item_at(2), Some(3));
        assert_eq!(q.move_target(2, 0), Some((3, None)));
        assert_eq!(q.item_at(5), None);
    }
}

#[cfg(test)]
mod reducer_tests {
    use super::tests::{queue, track};
    use super::*;
    use crate::cast::companion::PollOutcome;
    use crate::cast::timeline::{MusicTimeline, RemoteState};
    use RemoteState::*;

    fn at(state: RemoteState, item: i64, key: &str, time_ms: u64) -> PollOutcome {
        PollOutcome::Timeline(MusicTimeline {
            state,
            time_ms,
            duration_ms: 200_000,
            rating_key: Some(key.into()),
            play_queue_id: Some(9001),
            play_queue_item_id: Some(item),
            play_queue_version: Some(3),
        })
    }

    fn abc() -> CastQueue {
        queue(&[(5001, "a"), (5002, "b"), (5003, "c")])
    }

    fn idle() -> PollOutcome {
        PollOutcome::Timeline(MusicTimeline {
            state: Stopped,
            time_ms: 0,
            duration_ms: 0,
            rating_key: None,
            play_queue_id: None,
            play_queue_item_id: None,
            play_queue_version: None,
        })
    }

    fn foreign() -> PollOutcome {
        match at(Playing, 1, "z", 0) {
            PollOutcome::Timeline(mut t) => {
                t.play_queue_id = Some(4242);
                PollOutcome::Timeline(t)
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn first_timeline_emits_state_and_position() {
        let mut s = CastState::new();
        assert_eq!(
            s.apply(&at(Playing, 5002, "b", 61_250), &abc()),
            vec![
                CastEffect::EmitState {
                    status: "playing",
                    index: Some(1)
                },
                CastEffect::EmitPosition {
                    position: 61.25,
                    duration: 200.0
                },
            ]
        );
    }

    #[test]
    fn an_unchanged_timeline_emits_only_position() {
        let mut s = CastState::new();
        s.apply(&at(Playing, 5002, "b", 1_000), &abc());
        assert_eq!(
            s.apply(&at(Playing, 5002, "b", 2_000), &abc()),
            vec![CastEffect::EmitPosition {
                position: 2.0,
                duration: 200.0
            }]
        );
    }

    #[test]
    fn a_new_item_or_state_emits_state() {
        let mut s = CastState::new();
        s.apply(&at(Playing, 5001, "a", 1_000), &abc());
        let effects = s.apply(&at(Paused, 5002, "b", 0), &abc());
        assert_eq!(
            effects[0],
            CastEffect::EmitState {
                status: "paused",
                index: Some(1)
            }
        );
    }

    #[test]
    fn buffering_is_playing_with_the_buffering_flag() {
        let mut s = CastState::new();
        let effects = s.apply(&at(Buffering, 5001, "a", 0), &abc());
        assert!(effects.contains(&CastEffect::EmitState {
            status: "playing",
            index: Some(0)
        }));
        assert!(effects.contains(&CastEffect::EmitBuffering(true)));
        let effects = s.apply(&at(Playing, 5001, "a", 500), &abc());
        assert!(effects.contains(&CastEffect::EmitBuffering(false)));
    }

    #[test]
    fn crossing_ninety_percent_marks_played_once_per_item() {
        let mut s = CastState::new();
        s.apply(&at(Playing, 5001, "a", 100_000), &abc());
        let effects = s.apply(&at(Playing, 5001, "a", 180_000), &abc());
        assert!(effects.contains(&CastEffect::MarkPlayed("a".into())));
        let effects = s.apply(&at(Playing, 5001, "a", 190_000), &abc());
        assert!(!effects
            .iter()
            .any(|e| matches!(e, CastEffect::MarkPlayed(_))));
    }

    #[test]
    fn unknown_duration_never_marks_played() {
        let mut s = CastState::new();
        let mut q = abc();
        q.entries[0].track = track("a", 0.0);
        let outcome = match at(Playing, 5001, "a", 0) {
            PollOutcome::Timeline(mut t) => {
                t.duration_ms = 0;
                PollOutcome::Timeline(t)
            }
            _ => unreachable!(),
        };
        let effects = s.apply(&outcome, &q);
        assert!(!effects
            .iter()
            .any(|e| matches!(e, CastEffect::MarkPlayed(_))));
        assert!(effects.contains(&CastEffect::EmitPosition {
            position: 0.0,
            duration: 0.0
        }));
    }

    #[test]
    fn a_zero_duration_timeline_falls_back_to_the_track() {
        let mut s = CastState::new();
        let outcome = match at(Playing, 5001, "a", 190_000) {
            PollOutcome::Timeline(mut t) => {
                t.duration_ms = 0;
                PollOutcome::Timeline(t)
            }
            _ => unreachable!(),
        };
        let effects = s.apply(&outcome, &abc());
        assert!(effects.contains(&CastEffect::EmitPosition {
            position: 190.0,
            duration: 200.0
        }));
        assert!(effects.contains(&CastEffect::MarkPlayed("a".into())));
    }

    #[test]
    fn a_newer_version_requests_a_refetch() {
        let mut s = CastState::new();
        let outcome = match at(Playing, 5001, "a", 0) {
            PollOutcome::Timeline(mut t) => {
                t.play_queue_version = Some(4);
                PollOutcome::Timeline(t)
            }
            _ => unreachable!(),
        };
        assert!(s
            .apply(&outcome, &abc())
            .contains(&CastEffect::RefetchQueue));
    }

    #[test]
    fn an_unknown_item_refetches_and_keeps_the_last_index() {
        let mut s = CastState::new();
        s.apply(&at(Playing, 5002, "b", 0), &abc());
        let effects = s.apply(&at(Playing, 5009, "x", 0), &abc());
        assert!(effects.contains(&CastEffect::RefetchQueue));
        assert!(!effects
            .iter()
            .any(|e| matches!(e, CastEffect::EmitState { .. })));
        assert_eq!(s.index(), Some(1));
    }

    #[test]
    fn remap_moves_the_index_after_a_refetch() {
        let mut s = CastState::new();
        s.apply(&at(Playing, 5002, "b", 0), &abc());
        let edited = queue(&[(5000, "new"), (5001, "a"), (5002, "b"), (5003, "c")]);
        assert_eq!(
            s.remap(&edited),
            vec![CastEffect::EmitState {
                status: "playing",
                index: Some(2)
            }]
        );
    }

    #[test]
    fn three_failures_lose_the_link_and_one_success_restores_it() {
        let mut s = CastState::new();
        s.apply(&at(Playing, 5001, "a", 0), &abc());
        assert!(s.apply(&PollOutcome::Failed, &abc()).is_empty());
        assert!(s.apply(&PollOutcome::Failed, &abc()).is_empty());
        assert_eq!(
            s.apply(&PollOutcome::Failed, &abc()),
            vec![CastEffect::Link(Link::Lost)]
        );
        assert_eq!(s.link(), Link::Lost);
        let effects = s.apply(&at(Playing, 5001, "a", 9_000), &abc());
        assert_eq!(effects[0], CastEffect::Link(Link::Connected));
        // Everything is re-emitted after a gap.
        assert!(effects.contains(&CastEffect::EmitState {
            status: "playing",
            index: Some(0)
        }));
    }

    #[test]
    fn foreign_queue_before_ours_waits_out_the_grace() {
        let mut s = CastState::new();
        for _ in 0..FOREIGN_GRACE_POLLS - 1 {
            assert!(s.apply(&foreign(), &abc()).is_empty());
        }
        assert_eq!(s.apply(&foreign(), &abc()), vec![CastEffect::TakenOver]);
    }

    #[test]
    fn idle_before_ours_emits_nothing() {
        let mut s = CastState::new();
        assert!(s.apply(&idle(), &abc()).is_empty());
        assert!(s.apply(&PollOutcome::NoMusicTimeline, &abc()).is_empty());
    }

    #[test]
    fn foreign_queue_after_ours_is_a_takeover_at_once() {
        let mut s = CastState::new();
        s.apply(&at(Playing, 5001, "a", 0), &abc());
        assert_eq!(s.apply(&foreign(), &abc()), vec![CastEffect::TakenOver]);
        assert!(s.apply(&foreign(), &abc()).is_empty());
    }

    #[test]
    fn idle_after_ours_keeps_the_last_track() {
        let mut s = CastState::new();
        s.apply(&at(Playing, 5003, "c", 199_000), &abc());
        assert_eq!(
            s.apply(&idle(), &abc()),
            vec![CastEffect::EmitState {
                status: "stopped",
                index: Some(2)
            }]
        );
        assert_eq!(s.remote_state(), Some(Stopped));
    }

    #[test]
    fn a_duplicate_track_is_located_by_item_id() {
        let mut s = CastState::new();
        let q = queue(&[(1, "a"), (2, "b"), (3, "a")]);
        let effects = s.apply(&at(Playing, 3, "a", 0), &q);
        assert_eq!(
            effects[0],
            CastEffect::EmitState {
                status: "playing",
                index: Some(2)
            }
        );
    }

    #[test]
    fn restart_forgets_the_old_queue() {
        let mut s = CastState::new();
        s.apply(&at(Playing, 5001, "a", 0), &abc());
        s.restart();
        // The old queue now reads as foreign, and is waited out.
        let mut next = abc();
        next.play_queue_id = 9500;
        assert!(s.apply(&at(Playing, 5001, "a", 0), &next).is_empty());
    }

    #[test]
    fn local_resume_follows_the_remote_state() {
        let mut s = CastState::new();
        assert_eq!(s.local_resume(&CastQueue::default()), None);
        s.apply(&at(Playing, 5002, "b", 42_000), &abc());
        assert_eq!(
            s.local_resume(&abc()),
            Some(LocalResume::Play {
                index: 1,
                position: 42.0
            })
        );
        s.apply(&at(Paused, 5002, "b", 43_000), &abc());
        assert_eq!(
            s.local_resume(&abc()),
            Some(LocalResume::Paused {
                index: 1,
                position: 43.0
            })
        );
        s.apply(&idle(), &abc());
        assert_eq!(
            s.local_resume(&abc()),
            Some(LocalResume::Paused {
                index: 1,
                position: 0.0
            })
        );
    }

    #[test]
    fn local_resume_after_a_failed_poll_uses_the_last_timeline() {
        let mut s = CastState::new();
        s.apply(&at(Playing, 5003, "c", 12_000), &abc());
        s.apply(&PollOutcome::Failed, &abc());
        assert_eq!(
            s.local_resume(&abc()),
            Some(LocalResume::Play {
                index: 2,
                position: 12.0
            })
        );
        assert_eq!(
            LocalResume::Play {
                index: 2,
                position: 12.0
            }
            .paused(),
            LocalResume::Paused {
                index: 2,
                position: 12.0
            }
        );
    }

    #[test]
    fn poll_delay_backs_off_while_lost() {
        assert_eq!(poll_delay(Link::Connected, 0, false), POLL_VISIBLE);
        assert_eq!(poll_delay(Link::Connected, 0, true), POLL_HIDDEN);
        assert_eq!(
            poll_delay(Link::Lost, 3, false),
            std::time::Duration::from_secs(1)
        );
        assert_eq!(
            poll_delay(Link::Lost, 4, false),
            std::time::Duration::from_secs(2)
        );
        assert_eq!(
            poll_delay(Link::Lost, 6, true),
            std::time::Duration::from_secs(8)
        );
        assert_eq!(poll_delay(Link::Lost, 40, false), POLL_BACKOFF_MAX);
    }
}
