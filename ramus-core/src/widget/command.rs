use std::collections::HashSet;

use rand::Rng;
use serde::Deserialize;

use crate::models::Track;
use crate::util::shuffle_tracks;

/// What the widget was showing when its button was pressed, so a command
/// can be checked against what is really playing.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Expect {
    pub rating_key: Option<String>,
    pub revision: u64,
}

/// A command sent by a widget button, as JSON tagged by `action`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum WidgetCommand {
    /// `play: true` asks for playback, `false` for a pause; neither toggles,
    /// so a repeated tap can't undo the first.
    Play {
        play: bool,
        expect: Expect,
    },
    Next {
        expect: Expect,
    },
    Previous {
        expect: Expect,
    },
    /// Seek to a fraction (0..=1) of the track the widget was showing.
    Seek {
        fraction: f64,
        expect: Expect,
    },
    Resume,
    RandomFavouriteAlbum,
    ShuffleFavouriteSongs,
    RandomUnplayedAlbum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleAction {
    FavouriteAlbum,
    FavouriteSongs,
    UnplayedAlbum,
}

/// What to do with a widget command given the player's real state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dispatch {
    Play,
    Pause,
    Next,
    Previous,
    /// Seek to this position in seconds.
    SeekTo(f64),
    Idle(IdleAction),
    /// Nothing to do: the command is already satisfied or no longer applies.
    Dropped,
}

/// Decide what a widget command does against what is actually playing.
/// Transport acts on the real state whatever the widget showed; a seek is
/// only meaningful on the track it was aimed at, so it is dropped when the
/// track has changed or its length is unknown.
pub fn plan_dispatch(
    cmd: &WidgetCommand,
    current_key: Option<&str>,
    is_playing: bool,
    duration_sec: f64,
) -> Dispatch {
    match cmd {
        WidgetCommand::Play { play: true, .. } | WidgetCommand::Resume => {
            if is_playing || current_key.is_none() {
                Dispatch::Dropped
            } else {
                Dispatch::Play
            }
        }
        WidgetCommand::Play { play: false, .. } => {
            if is_playing {
                Dispatch::Pause
            } else {
                Dispatch::Dropped
            }
        }
        WidgetCommand::Next { .. } => Dispatch::Next,
        WidgetCommand::Previous { .. } => Dispatch::Previous,
        WidgetCommand::Seek { fraction, expect } => {
            if duration_sec <= 0.0 || expect.rating_key.as_deref() != current_key {
                Dispatch::Dropped
            } else {
                Dispatch::SeekTo(fraction.clamp(0.0, 1.0) * duration_sec)
            }
        }
        WidgetCommand::RandomFavouriteAlbum => Dispatch::Idle(IdleAction::FavouriteAlbum),
        WidgetCommand::ShuffleFavouriteSongs => Dispatch::Idle(IdleAction::FavouriteSongs),
        WidgetCommand::RandomUnplayedAlbum => Dispatch::Idle(IdleAction::UnplayedAlbum),
    }
}

/// The queue for "shuffle favourite songs": every favourite, limited to
/// downloaded tracks when `downloaded` is given (offline, where anything
/// else can't play), shuffled with artists spread out.
pub fn favourite_songs_queue(
    tracks: Vec<Track>,
    downloaded: Option<&HashSet<String>>,
    rng: &mut impl Rng,
) -> Vec<Track> {
    let tracks = match downloaded {
        Some(keys) => tracks
            .into_iter()
            .filter(|t| keys.contains(&t.rating_key))
            .collect(),
        None => tracks,
    };
    shuffle_tracks(tracks, rng)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};

    fn expect(key: &str) -> Expect {
        Expect {
            rating_key: Some(key.into()),
            revision: 3,
        }
    }

    fn track(key: &str, artist: &str) -> Track {
        Track {
            rating_key: key.into(),
            title: "Song".into(),
            artist_name: artist.into(),
            track_artist: None,
            album_title: "Album".into(),
            album_key: None,
            index: None,
            duration: 0.0,
            codec: None,
            part_key: None,
            thumb: None,
            is_favourite: true,
            bitrate: None,
            disc_number: None,
            file_size_bytes: None,
            rating_count: None,
            view_count: None,
            last_viewed_at: None,
        }
    }

    #[test]
    fn parses_seek_json() {
        let cmd: WidgetCommand = serde_json::from_str(
            r#"{"action":"seek","fraction":0.5,"expect":{"ratingKey":"9","revision":3}}"#,
        )
        .unwrap();
        assert_eq!(
            cmd,
            WidgetCommand::Seek {
                fraction: 0.5,
                expect: expect("9")
            }
        );
        let idle: WidgetCommand =
            serde_json::from_str(r#"{"action":"randomUnplayedAlbum"}"#).unwrap();
        assert_eq!(idle, WidgetCommand::RandomUnplayedAlbum);
    }

    #[test]
    fn second_play_tap_is_dropped() {
        let play = WidgetCommand::Play {
            play: true,
            expect: expect("9"),
        };
        assert_eq!(
            plan_dispatch(&play, Some("9"), false, 200.0),
            Dispatch::Play
        );
        assert_eq!(
            plan_dispatch(&play, Some("9"), true, 200.0),
            Dispatch::Dropped
        );
        let pause = WidgetCommand::Play {
            play: false,
            expect: expect("9"),
        };
        assert_eq!(
            plan_dispatch(&pause, Some("9"), true, 200.0),
            Dispatch::Pause
        );
        assert_eq!(
            plan_dispatch(&pause, Some("9"), false, 200.0),
            Dispatch::Dropped
        );
    }

    #[test]
    fn seek_on_other_track_is_dropped() {
        let seek = WidgetCommand::Seek {
            fraction: 0.25,
            expect: expect("9"),
        };
        assert_eq!(
            plan_dispatch(&seek, Some("10"), true, 200.0),
            Dispatch::Dropped
        );
        assert_eq!(
            plan_dispatch(&seek, Some("9"), true, 200.0),
            Dispatch::SeekTo(50.0)
        );
        let past_end = WidgetCommand::Seek {
            fraction: 1.7,
            expect: expect("9"),
        };
        assert_eq!(
            plan_dispatch(&past_end, Some("9"), true, 200.0),
            Dispatch::SeekTo(200.0)
        );
    }

    #[test]
    fn seek_with_unknown_duration_is_dropped() {
        let seek = WidgetCommand::Seek {
            fraction: 0.25,
            expect: expect("9"),
        };
        assert_eq!(
            plan_dispatch(&seek, Some("9"), true, 0.0),
            Dispatch::Dropped
        );
    }

    #[test]
    fn next_ignores_expectation() {
        let next = WidgetCommand::Next {
            expect: expect("9"),
        };
        assert_eq!(
            plan_dispatch(&next, Some("10"), true, 200.0),
            Dispatch::Next
        );
        let prev = WidgetCommand::Previous {
            expect: expect("9"),
        };
        assert_eq!(plan_dispatch(&prev, None, false, 0.0), Dispatch::Previous);
    }

    #[test]
    fn resume_without_track_is_dropped() {
        assert_eq!(
            plan_dispatch(&WidgetCommand::Resume, None, false, 0.0),
            Dispatch::Dropped
        );
        assert_eq!(
            plan_dispatch(&WidgetCommand::Resume, Some("9"), false, 200.0),
            Dispatch::Play
        );
        assert_eq!(
            plan_dispatch(&WidgetCommand::Resume, Some("9"), true, 200.0),
            Dispatch::Dropped
        );
        assert_eq!(
            plan_dispatch(&WidgetCommand::ShuffleFavouriteSongs, None, false, 0.0),
            Dispatch::Idle(IdleAction::FavouriteSongs)
        );
    }

    #[test]
    fn favourite_songs_offline_keeps_only_downloaded() {
        let tracks = vec![track("1", "A"), track("2", "B"), track("3", "A")];
        let downloaded: HashSet<String> = ["1", "3"].iter().map(|s| s.to_string()).collect();
        let mut rng = StdRng::seed_from_u64(2);
        let mut keys: Vec<String> =
            favourite_songs_queue(tracks.clone(), Some(&downloaded), &mut rng)
                .into_iter()
                .map(|t| t.rating_key)
                .collect();
        keys.sort();
        assert_eq!(keys, vec!["1", "3"]);
        assert_eq!(favourite_songs_queue(tracks, None, &mut rng).len(), 3);
    }

    #[test]
    fn favourite_songs_empty_stays_empty() {
        let mut rng = StdRng::seed_from_u64(2);
        assert!(favourite_songs_queue(Vec::new(), None, &mut rng).is_empty());
        let none: HashSet<String> = HashSet::new();
        assert!(favourite_songs_queue(vec![track("1", "A")], Some(&none), &mut rng).is_empty());
    }
}
